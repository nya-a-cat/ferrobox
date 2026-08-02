use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use ferrobox_core::{
    CreateSnapshotRequest, DirectoryEntry, ExecRequest, ExecResult, ExecTermination, FileKind,
    ListDirectoryRequest, ListDirectoryResult, OutputTruncation, ProcessId, ReadFileRequest,
    ReadFileResult, RuntimeError, RuntimeErrorKind, SandboxHandle, SandboxId, SandboxPath,
    SandboxRuntime, SandboxSpec, SandboxState, SignalRequest, SignalResult, SnapshotHandle,
    SnapshotId, SnapshotVerification, WriteFileRequest, WriteFileResult,
};
use ferrobox_protocol::guest::v1::{
    self as guest, Auth, HealthRequest, InitRequest, ListDirectoryRequest as GuestListRequest,
    ReadFileRequest as GuestReadRequest, RekeyRequest, SignalProcessRequest as GuestSignalRequest,
    StartProcessRequest, WriteFileRequest as GuestWriteRequest, process_event,
};
use tokio::{
    fs,
    process::{Child, Command},
    sync::{Mutex, RwLock},
    time::{Instant, sleep, timeout},
};
use tonic::Request;

use crate::{
    firecracker::{
        BootSource, Drive, FirecrackerClient, InstanceAction, InstanceActionType, MachineConfig,
        MemoryBackend, MemoryBackendType, NetworkInterface, SnapshotCreate, SnapshotLoad,
        SnapshotType, VersionResponse, VmState, VmStateValue, Vsock,
    },
    network::{NetworkLease, NetworkManager},
    rootfs::{clone_readonly_asset, clone_rootfs, verify_regular_file},
    snapshot::{SnapshotArtifact, SnapshotStageRequest, SnapshotStore},
    vsock::GuestConnector,
};

const FIRECRACKER_VERSION: &str = "1.16.1";

#[derive(Clone, Debug)]
pub struct FirecrackerRuntimeConfig {
    pub firecracker_binary: PathBuf,
    pub jailer_binary: PathBuf,
    pub kernel_image: PathBuf,
    pub rootfs_template: PathBuf,
    pub snapshot_root: Option<PathBuf>,
    pub chroot_base: PathBuf,
    pub runtime_root: PathBuf,
    pub jail_uid: u32,
    pub jail_gid: u32,
    pub guest_port: u32,
    pub api_timeout: Duration,
    pub boot_timeout: Duration,
    pub node_id: String,
}

impl FirecrackerRuntimeConfig {
    pub async fn validate(&self) -> Result<(), RuntimeError> {
        if !cfg!(target_os = "linux") {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unsupported,
                "Firecracker requires Linux",
            ));
        }
        for path in [
            &self.firecracker_binary,
            &self.jailer_binary,
            &self.kernel_image,
            &self.rootfs_template,
        ] {
            verify_regular_file(path)
                .await
                .map_err(|error| RuntimeError::invalid(error.to_string()))?;
        }
        if !self.chroot_base.is_absolute() || !self.runtime_root.is_absolute() {
            return Err(RuntimeError::invalid("runtime paths must be absolute"));
        }
        if self
            .snapshot_root
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            return Err(RuntimeError::invalid("snapshot root must be absolute"));
        }
        Ok(())
    }
}

struct FirecrackerSandbox {
    state: SandboxState,
    spec: SandboxSpec,
    child: Child,
    api: FirecrackerClient,
    guest_client: guest::guest_service_client::GuestServiceClient<tonic::transport::Channel>,
    guest_token: String,
    chroot_root: PathBuf,
    network: Option<NetworkLease>,
}

pub struct FirecrackerRuntime {
    config: FirecrackerRuntimeConfig,
    network: NetworkManager,
    snapshot_store: SnapshotStore,
    sandboxes: RwLock<HashMap<SandboxId, Arc<Mutex<FirecrackerSandbox>>>>,
    snapshots: RwLock<HashMap<SnapshotId, Arc<Mutex<SnapshotArtifact>>>>,
    ready_pool: Mutex<Vec<SandboxHandle>>,
}

#[derive(Clone, Debug)]
struct RestoreAssets {
    vmstate_path: PathBuf,
    memory_path: PathBuf,
    rootfs_path: PathBuf,
    captured_guest_token: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct GuestNetworkConfig {
    guest_ipv4: String,
    guest_prefix_length: u32,
    gateway_ipv4: String,
    dns_ipv4: String,
}

impl GuestNetworkConfig {
    fn from_lease(network: Option<&NetworkLease>) -> Self {
        let Some(lease) = network else {
            return Self::default();
        };
        Self {
            guest_ipv4: lease.guest_address.clone(),
            guest_prefix_length: 24,
            gateway_ipv4: lease.gateway.clone(),
            dns_ipv4: lease.dns_ipv4.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ExecutionTimings {
    pub validation_us: u128,
    pub guest_lookup_us: u128,
    pub start_rpc_us: u128,
    pub stream_us: u128,
    pub total_us: u128,
}

impl FirecrackerRuntime {
    pub async fn new(config: FirecrackerRuntimeConfig) -> Result<Self, RuntimeError> {
        config.validate().await?;
        fs::create_dir_all(&config.chroot_base)
            .await
            .map_err(|error| RuntimeError::internal(format!("create chroot base: {error}")))?;
        fs::create_dir_all(&config.runtime_root)
            .await
            .map_err(|error| RuntimeError::internal(format!("create runtime root: {error}")))?;
        let snapshot_store = SnapshotStore::new(
            config.runtime_root.join("snapshots"),
            config.node_id.clone(),
            FIRECRACKER_VERSION,
            &config.kernel_image,
        )
        .await?;
        Ok(Self {
            config,
            network: NetworkManager,
            snapshot_store,
            sandboxes: RwLock::new(HashMap::new()),
            snapshots: RwLock::new(HashMap::new()),
            ready_pool: Mutex::new(Vec::new()),
        })
    }

    pub async fn prewarm(
        &self,
        spec: SandboxSpec,
        count: usize,
    ) -> Result<Vec<u128>, RuntimeError> {
        if spec.template_id != "python"
            || spec.cpu_count != 1
            || spec.memory_mb != 512
            || spec.network != ferrobox_core::NetworkMode::Disabled
        {
            return Err(RuntimeError::invalid(
                "ready pool supports 1 vCPU, 512 MiB, and disabled networking",
            ));
        }
        let mut samples = Vec::with_capacity(count);
        if count > 0 && !self.snapshot_available().await {
            let started = std::time::Instant::now();
            let handle = self.create_fresh(spec.clone()).await?;
            if let Err(error) = self.warm_ready_sandbox(&handle).await {
                let _ = <Self as SandboxRuntime>::delete(self, &handle.sandbox_id).await;
                return Err(error);
            }
            samples.push(started.elapsed().as_micros());
            self.ready_pool.lock().await.push(handle);
        }
        let remaining = count.saturating_sub(samples.len());
        let prepared = futures::future::try_join_all((0..remaining).map(|_| {
            let spec = spec.clone();
            async move {
                let started = std::time::Instant::now();
                let handle = self.create_fresh(spec).await?;
                if let Err(error) = self.warm_ready_sandbox(&handle).await {
                    let _ = <Self as SandboxRuntime>::delete(self, &handle.sandbox_id).await;
                    return Err(error);
                }
                Ok::<_, RuntimeError>((handle, started.elapsed().as_micros()))
            }
        }))
        .await?;
        let mut pool = self.ready_pool.lock().await;
        for (handle, elapsed) in prepared {
            pool.push(handle);
            samples.push(elapsed);
        }
        Ok(samples)
    }

    pub async fn ready_pool_len(&self) -> usize {
        self.ready_pool.lock().await.len()
    }

    pub async fn firecracker_rss_kib(&self) -> Result<u64, RuntimeError> {
        let mut processes = fs::read_dir("/proc")
            .await
            .map_err(|error| RuntimeError::internal(format!("read /proc: {error}")))?;
        let mut total = 0_u64;
        let mut count = 0_usize;
        while let Some(entry) = processes
            .next_entry()
            .await
            .map_err(|error| RuntimeError::internal(format!("read /proc entry: {error}")))?
        {
            let process_id = entry.file_name().to_string_lossy().parse::<u32>().ok();
            let Some(process_id) = process_id else {
                continue;
            };
            let process_name = fs::read_to_string(format!("/proc/{process_id}/comm"))
                .await
                .unwrap_or_default();
            if process_name.trim() != "firecracker" {
                continue;
            }
            let status = fs::read_to_string(format!("/proc/{process_id}/status"))
                .await
                .map_err(|error| {
                    RuntimeError::internal(format!("read Firecracker process status: {error}"))
                })?;
            total = total.saturating_add(
                parse_vm_rss_kib(&status)
                    .ok_or_else(|| RuntimeError::internal("Firecracker VmRSS is missing"))?,
            );
            count += 1;
        }
        if count == 0 {
            return Err(RuntimeError::internal(
                "no Firecracker processes found for RSS measurement",
            ));
        }
        Ok(total)
    }

    async fn warm_ready_sandbox(&self, handle: &SandboxHandle) -> Result<(), RuntimeError> {
        for argv in [
            vec!["/bin/true".to_owned()],
            vec![
                "python3".to_owned(),
                "-c".to_owned(),
                "print(42)".to_owned(),
            ],
        ] {
            let result = <Self as SandboxRuntime>::execute(
                self,
                &handle.sandbox_id,
                ExecRequest {
                    argv,
                    cwd: SandboxPath::workspace(),
                    environment: Default::default(),
                    timeout_seconds: 30,
                    max_output_bytes: 1024,
                },
            )
            .await?;
            if result.termination != (ExecTermination::Exited { exit_code: 0 }) {
                return Err(RuntimeError::new(
                    RuntimeErrorKind::Unavailable,
                    format!("ready-pool warmup failed: {:?}", result.termination),
                ));
            }
        }
        Ok(())
    }

    fn jail_root(&self, id: &SandboxId) -> Result<PathBuf, RuntimeError> {
        let executable_name = self
            .config
            .firecracker_binary
            .file_name()
            .ok_or_else(|| RuntimeError::invalid("firecracker binary has no filename"))?;
        Ok(self
            .config
            .chroot_base
            .join(executable_name)
            .join(id.to_string())
            .join("root"))
    }

    async fn cleanup_chroot(&self, chroot_root: &Path) -> Result<(), RuntimeError> {
        if !chroot_root.starts_with(&self.config.chroot_base)
            || chroot_root == self.config.chroot_base
        {
            return Err(RuntimeError::internal("refusing unsafe chroot cleanup"));
        }
        if fs::metadata(chroot_root).await.is_ok() {
            fs::remove_dir_all(chroot_root)
                .await
                .map_err(|error| RuntimeError::internal(format!("remove jail: {error}")))?;
        }
        Ok(())
    }

    async fn terminate_record(
        &self,
        record: &mut FirecrackerSandbox,
    ) -> Result<(), RuntimeError> {
        record.state = SandboxState::Deleting;
        let _ = record
            .api
            .put(
                "/actions",
                &InstanceAction {
                    action_type: InstanceActionType::SendCtrlAltDel,
                },
            )
            .await;
        match timeout(Duration::from_secs(2), record.child.wait()).await {
            Ok(Ok(_)) => {}
            Ok(Err(_)) | Err(_) => {
                let _ = record.child.kill().await;
                let _ = record.child.wait().await;
            }
        }

        let network_result = if let Some(lease) = &record.network {
            self.network.delete(lease).await
        } else {
            Ok(())
        };
        let jail_result = self.cleanup_chroot(&record.chroot_root).await;
        if network_result.is_ok() && jail_result.is_ok() {
            record.state = SandboxState::Deleted;
        } else {
            record.state = SandboxState::Failed;
        }
        network_result?;
        jail_result
    }

    async fn wait_for_socket(path: &Path, limit: Duration) -> Result<(), RuntimeError> {
        let started = Instant::now();
        while started.elapsed() < limit {
            if fs::metadata(path).await.is_ok() {
                return Ok(());
            }
            sleep(Duration::from_millis(5)).await;
        }
        Err(RuntimeError::new(
            RuntimeErrorKind::Timeout,
            format!("socket did not appear: {}", path.display()),
        ))
    }

    async fn configure_and_start(
        &self,
        api: &FirecrackerClient,
        spec: &SandboxSpec,
        network: Option<&NetworkLease>,
    ) -> Result<(), RuntimeError> {
        let version: VersionResponse = api.get("/version").await.map_err(fc_error)?;
        if version.firecracker_version != FIRECRACKER_VERSION {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unavailable,
                format!(
                    "expected Firecracker {FIRECRACKER_VERSION}, got {}",
                    version.firecracker_version
                ),
            ));
        }
        api.put(
            "/machine-config",
            &MachineConfig {
                vcpu_count: spec.cpu_count,
                mem_size_mib: spec.memory_mb,
                smt: false,
                track_dirty_pages: false,
            },
        )
        .await
        .map_err(fc_error)?;
        api.put(
            "/boot-source",
            &BootSource {
                kernel_image_path: "/kernel/vmlinux".to_owned(),
                boot_args: "console=ttyS0 reboot=k panic=1 pci=off root=/dev/vda rw".to_owned(),
            },
        )
        .await
        .map_err(fc_error)?;
        api.put(
            "/drives/rootfs",
            &Drive {
                drive_id: "rootfs".to_owned(),
                path_on_host: "/images/rootfs.ext4".to_owned(),
                is_root_device: true,
                is_read_only: false,
            },
        )
        .await
        .map_err(fc_error)?;
        if let Some(lease) = network {
            api.put(
                "/network-interfaces/net1",
                &NetworkInterface {
                    iface_id: "net1".to_owned(),
                    host_dev_name: lease.tap_name.clone(),
                    guest_mac: lease.guest_mac.clone(),
                },
            )
            .await
            .map_err(fc_error)?;
        }
        api.put(
            "/vsock",
            &Vsock {
                guest_cid: guest_cid(spec),
                uds_path: "/run/guest.vsock".to_owned(),
            },
        )
        .await
        .map_err(fc_error)?;
        api.put(
            "/actions",
            &InstanceAction {
                action_type: InstanceActionType::InstanceStart,
            },
        )
        .await
        .map_err(fc_error)
    }

    async fn load_snapshot(&self, api: &FirecrackerClient) -> Result<(), RuntimeError> {
        let version: VersionResponse = api.get("/version").await.map_err(fc_error)?;
        if version.firecracker_version != FIRECRACKER_VERSION {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unavailable,
                format!(
                    "expected Firecracker {FIRECRACKER_VERSION}, got {}",
                    version.firecracker_version
                ),
            ));
        }
        api.put(
            "/snapshot/load",
            &SnapshotLoad {
                snapshot_path: "/snapshot/vmstate".to_owned(),
                mem_backend: MemoryBackend {
                    backend_path: "/snapshot/memory".to_owned(),
                    backend_type: MemoryBackendType::File,
                },
                track_dirty_pages: false,
                resume_vm: true,
            },
        )
        .await
        .map_err(fc_error)
    }

    async fn wait_for_guest(
        &self,
        connector: &GuestConnector,
    ) -> Result<
        guest::guest_service_client::GuestServiceClient<tonic::transport::Channel>,
        RuntimeError,
    > {
        let started = Instant::now();
        loop {
            if started.elapsed() >= self.config.boot_timeout {
                return Err(RuntimeError::new(
                    RuntimeErrorKind::Timeout,
                    "guest agent did not become ready",
                ));
            }
            if let Ok(mut client) = connector.client().await
                && client.health(Request::new(HealthRequest {})).await.is_ok()
            {
                return Ok(client);
            }
            sleep(Duration::from_millis(5)).await;
        }
    }

    async fn initialize_guest(
        &self,
        id: &SandboxId,
        connector: &GuestConnector,
        token_value: &str,
        network: Option<&NetworkLease>,
    ) -> Result<
        guest::guest_service_client::GuestServiceClient<tonic::transport::Channel>,
        RuntimeError,
    > {
        let started = Instant::now();
        loop {
            if started.elapsed() >= self.config.boot_timeout {
                return Err(RuntimeError::new(
                    RuntimeErrorKind::Timeout,
                    "guest agent did not become ready",
                ));
            }
            if let Ok(mut client) = self.wait_for_guest(connector).await {
                let response = client
                    .init(Request::new(InitRequest {
                        request_id: uuid::Uuid::new_v4().to_string(),
                        sandbox_id: id.to_string(),
                        token: token_value.to_owned(),
                        command_uid: 1000,
                        command_gid: 1000,
                        max_file_bytes: ferrobox_core::MAX_FILE_BYTES,
                        max_processes: 256,
                        guest_ipv4: network
                            .map_or_else(String::new, |lease| lease.guest_address.clone()),
                        guest_prefix_length: if network.is_some() { 24 } else { 0 },
                        gateway_ipv4: network
                            .map_or_else(String::new, |lease| lease.gateway.clone()),
                        dns_ipv4: if network.is_some() {
                            network
                                .map(|lease| lease.dns_ipv4.clone())
                                .unwrap_or_default()
                        } else {
                            String::new()
                        },
                    }))
                    .await;
                match response {
                    Ok(_) => return Ok(client),
                    Err(error) if error.code() == tonic::Code::Unavailable => {}
                    Err(error) => {
                        return Err(RuntimeError::new(
                            RuntimeErrorKind::Unavailable,
                            format!("guest init: {error}"),
                        ));
                    }
                }
            }
            sleep(Duration::from_millis(5)).await;
        }
    }

    async fn rekey_guest(
        &self,
        id: &SandboxId,
        connector: &GuestConnector,
        previous_token: &str,
        token_value: &str,
        network: Option<&NetworkLease>,
    ) -> Result<
        guest::guest_service_client::GuestServiceClient<tonic::transport::Channel>,
        RuntimeError,
    > {
        let mut client = self.wait_for_guest(connector).await?;
        let network = GuestNetworkConfig::from_lease(network);
        self.rekey_connected_guest(
            id,
            &mut client,
            previous_token,
            token_value,
            &network,
        )
        .await?;
        Ok(client)
    }

    async fn rekey_connected_guest(
        &self,
        id: &SandboxId,
        client: &mut guest::guest_service_client::GuestServiceClient<tonic::transport::Channel>,
        previous_token: &str,
        token_value: &str,
        network: &GuestNetworkConfig,
    ) -> Result<(), RuntimeError> {
        client
            .rekey(Request::new(RekeyRequest {
                auth: Some(Auth {
                    token: previous_token.to_owned(),
                }),
                request_id: uuid::Uuid::new_v4().to_string(),
                sandbox_id: id.to_string(),
                token: token_value.to_owned(),
                command_uid: 1000,
                command_gid: 1000,
                max_file_bytes: ferrobox_core::MAX_FILE_BYTES,
                max_processes: 256,
                guest_ipv4: network.guest_ipv4.clone(),
                guest_prefix_length: network.guest_prefix_length,
                gateway_ipv4: network.gateway_ipv4.clone(),
                dns_ipv4: network.dns_ipv4.clone(),
            }))
            .await
            .map_err(guest_error)?;
        Ok(())
    }

    fn snapshot_paths(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        (
            root.join("vmstate"),
            root.join("memory"),
            root.join("rootfs.ext4"),
            root.join("READY"),
        )
    }

    async fn snapshot_available(&self) -> bool {
        let Some(root) = &self.config.snapshot_root else {
            return false;
        };
        let (state, memory, rootfs, ready) = Self::snapshot_paths(root);
        let ready_version_matches = fs::read_to_string(&ready)
            .await
            .is_ok_and(|version| version == FIRECRACKER_VERSION);
        fs::metadata(state).await.is_ok()
            && fs::metadata(memory).await.is_ok()
            && fs::metadata(rootfs).await.is_ok()
            && ready_version_matches
    }

    async fn capture_snapshot(
        &self,
        api: &FirecrackerClient,
        rootfs_path: &Path,
        jail_snapshot_path: &Path,
    ) -> Result<(), RuntimeError> {
        let Some(snapshot_root) = &self.config.snapshot_root else {
            return Ok(());
        };
        api.patch(
            "/vm",
            &VmState {
                state: VmStateValue::Paused,
            },
        )
        .await
        .map_err(fc_error)?;
        api.put(
            "/snapshot/create",
            &SnapshotCreate {
                snapshot_type: SnapshotType::Full,
                snapshot_path: "/snapshot/vmstate".to_owned(),
                mem_file_path: "/snapshot/memory".to_owned(),
            },
        )
        .await
        .map_err(fc_error)?;

        fs::create_dir_all(snapshot_root)
            .await
            .map_err(|error| RuntimeError::internal(format!("create snapshot root: {error}")))?;
        let (state, memory, rootfs, ready) = Self::snapshot_paths(snapshot_root);
        fs::copy(jail_snapshot_path.join("vmstate"), &state)
            .await
            .map_err(|error| RuntimeError::internal(format!("copy snapshot state: {error}")))?;
        fs::copy(jail_snapshot_path.join("memory"), &memory)
            .await
            .map_err(|error| RuntimeError::internal(format!("copy snapshot memory: {error}")))?;
        let readonly_status = Command::new("chmod")
            .arg("0444")
            .arg(&state)
            .arg(&memory)
            .status()
            .await
            .map_err(|error| RuntimeError::internal(format!("start snapshot chmod: {error}")))?;
        if !readonly_status.success() {
            return Err(RuntimeError::internal("chmod snapshot assets failed"));
        }
        clone_rootfs(rootfs_path, &rootfs)
            .await
            .map_err(|error| RuntimeError::internal(format!("copy snapshot rootfs: {error}")))?;
        fs::write(&ready, FIRECRACKER_VERSION)
            .await
            .map_err(|error| RuntimeError::internal(format!("mark snapshot ready: {error}")))?;
        api.patch(
            "/vm",
            &VmState {
                state: VmStateValue::Resumed,
            },
        )
        .await
        .map_err(fc_error)
    }

    async fn record(&self, id: &SandboxId) -> Result<Arc<Mutex<FirecrackerSandbox>>, RuntimeError> {
        self.sandboxes
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeError::not_found("sandbox does not exist"))
    }

    async fn guest_client(
        &self,
        id: &SandboxId,
    ) -> Result<
        (
            guest::guest_service_client::GuestServiceClient<tonic::transport::Channel>,
            String,
        ),
        RuntimeError,
    > {
        let record = self.record(id).await?;
        let (client, token_value, state) = {
            let record = record.lock().await;
            (
                record.guest_client.clone(),
                record.guest_token.clone(),
                record.state,
            )
        };
        if state != SandboxState::Running {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "sandbox is not running",
            ));
        }
        Ok((client, token_value))
    }
}

impl FirecrackerRuntime {
    pub async fn benchmark_guest_lookup_us(
        &self,
        sandbox_id: &SandboxId,
    ) -> Result<u128, RuntimeError> {
        let started = std::time::Instant::now();
        let _ = self.guest_client(sandbox_id).await?;
        Ok(started.elapsed().as_micros())
    }

    async fn snapshot_record(
        &self,
        id: &SnapshotId,
    ) -> Result<Arc<Mutex<SnapshotArtifact>>, RuntimeError> {
        self.snapshots
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeError::not_found("snapshot does not exist"))
    }

    pub async fn benchmark_execute(
        &self,
        sandbox_id: &SandboxId,
        request: ExecRequest,
    ) -> Result<(ExecResult, ExecutionTimings), RuntimeError> {
        self.execute_measured(sandbox_id, request).await
    }

    pub async fn benchmark_raw_true_us(
        &self,
        sandbox_id: &SandboxId,
    ) -> Result<u128, RuntimeError> {
        let started = Instant::now();
        let (mut client, token_value) = self.guest_client(sandbox_id).await?;
        let response = client
            .start_process(Request::new(StartProcessRequest {
                auth: Some(Auth { token: token_value }),
                request_id: uuid::Uuid::new_v4().to_string(),
                argv: vec!["/bin/true".to_owned()],
                cwd: "/home/sandbox".to_owned(),
                environment: HashMap::new(),
                timeout_millis: 30_000,
                max_output_bytes: 1024 * 1024,
            }))
            .await
            .map_err(guest_error)?;
        let mut stream = response.into_inner();
        let mut successful_exit = false;
        while let Some(event) = stream.message().await.map_err(guest_error)? {
            match event.event {
                Some(process_event::Event::Exit(exit)) if exit.exit_code == Some(0) => {
                    successful_exit = true;
                }
                Some(process_event::Event::Exit(exit)) => {
                    return Err(RuntimeError::internal(format!(
                        "benchmark guest command failed: {exit:?}"
                    )));
                }
                Some(process_event::Event::Error(error)) => {
                    return Err(RuntimeError::new(
                        RuntimeErrorKind::Unavailable,
                        format!("guest {}: {}", error.code, error.message),
                    ));
                }
                _ => {}
            }
        }
        if !successful_exit {
            return Err(RuntimeError::internal(
                "benchmark guest command omitted a successful exit",
            ));
        }
        Ok(started.elapsed().as_micros())
    }

    async fn execute_measured(
        &self,
        sandbox_id: &SandboxId,
        request: ExecRequest,
    ) -> Result<(ExecResult, ExecutionTimings), RuntimeError> {
        let total_started = Instant::now();
        let validation_started = Instant::now();
        request
            .validate()
            .map_err(|error| RuntimeError::invalid(error.to_string()))?;
        let validation_us = validation_started.elapsed().as_micros();

        let lookup_started = Instant::now();
        let (mut client, token_value) = self.guest_client(sandbox_id).await?;
        let guest_lookup_us = lookup_started.elapsed().as_micros();

        let rpc_started = Instant::now();
        let response = client
            .start_process(Request::new(StartProcessRequest {
                auth: Some(Auth { token: token_value }),
                request_id: uuid::Uuid::new_v4().to_string(),
                argv: request.argv,
                cwd: request.cwd.to_string(),
                environment: request.environment.into_iter().collect(),
                timeout_millis: request.timeout_seconds.saturating_mul(1000),
                max_output_bytes: request.max_output_bytes,
            }))
            .await
            .map_err(guest_error)?;
        let start_rpc_us = rpc_started.elapsed().as_micros();

        let stream_started = Instant::now();
        let mut stream = response.into_inner();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut process_id = None;
        let mut termination = None;
        let mut truncation = OutputTruncation::default();
        while let Some(event) = stream.message().await.map_err(guest_error)? {
            if process_id.is_none() && !event.process_id.is_empty() {
                process_id = Some(
                    ProcessId::new(event.process_id)
                        .map_err(|error| RuntimeError::internal(error.to_string()))?,
                );
            }
            match event.event {
                Some(process_event::Event::Stdout(data)) => stdout.extend(data),
                Some(process_event::Event::Stderr(data)) => stderr.extend(data),
                Some(process_event::Event::Exit(exit)) => {
                    truncation = OutputTruncation {
                        stdout: exit.stdout_truncated,
                        stderr: exit.stderr_truncated,
                    };
                    termination = Some(if exit.timed_out {
                        ExecTermination::TimedOut
                    } else if exit.output_limit_exceeded {
                        ExecTermination::OutputLimitExceeded
                    } else if let Some(code) = exit.exit_code {
                        ExecTermination::Exited { exit_code: code }
                    } else {
                        ExecTermination::Signaled {
                            signal: exit.signal.unwrap_or_default(),
                        }
                    });
                }
                Some(process_event::Event::Error(error)) => {
                    return Err(RuntimeError::new(
                        RuntimeErrorKind::Unavailable,
                        format!("guest {}: {}", error.code, error.message),
                    ));
                }
                None => {}
            }
        }
        let stream_us = stream_started.elapsed().as_micros();
        let result = ExecResult {
            process_id: process_id
                .ok_or_else(|| RuntimeError::internal("guest omitted process id"))?,
            termination: termination
                .ok_or_else(|| RuntimeError::internal("guest omitted process exit"))?,
            stdout,
            stderr,
            truncation,
        };
        Ok((
            result,
            ExecutionTimings {
                validation_us,
                guest_lookup_us,
                start_rpc_us,
                stream_us,
                total_us: total_started.elapsed().as_micros(),
            },
        ))
    }

    async fn create_fresh(&self, spec: SandboxSpec) -> Result<SandboxHandle, RuntimeError> {
        spec.validate()
            .map_err(|error| RuntimeError::invalid(error.to_string()))?;
        let snapshot_compatible = spec.cpu_count == 1
            && spec.memory_mb == 512
            && spec.network == ferrobox_core::NetworkMode::Disabled;
        let restore = if snapshot_compatible && self.snapshot_available().await {
            self.config.snapshot_root.as_ref().map(|root| RestoreAssets {
                vmstate_path: root.join("vmstate"),
                memory_path: root.join("memory"),
                rootfs_path: root.join("rootfs.ext4"),
                captured_guest_token: None,
            })
        } else {
            None
        };
        let capture_template = snapshot_compatible
            && self.config.snapshot_root.is_some()
            && restore.is_none();
        self.launch(spec, SandboxId::new(), restore, capture_template)
            .await
    }

    async fn create_from_snapshot_artifact(
        &self,
        artifact: &SnapshotArtifact,
        captured_guest_token: String,
    ) -> Result<SandboxHandle, RuntimeError> {
        let restore = RestoreAssets {
            vmstate_path: artifact.vmstate_path(),
            memory_path: artifact.memory_path(),
            rootfs_path: artifact.rootfs_path(),
            captured_guest_token: Some(captured_guest_token),
        };
        self.launch(
            artifact.spec().clone(),
            SandboxId::new(),
            Some(restore),
            false,
        )
        .await
    }

    async fn launch(
        &self,
        spec: SandboxSpec,
        id: SandboxId,
        restore: Option<RestoreAssets>,
        capture_template: bool,
    ) -> Result<SandboxHandle, RuntimeError> {
        let chroot_root = self.jail_root(&id)?;
        let result = self
            .launch_inner(spec, id, restore, capture_template)
            .await;
        if result.is_err()
            && let Err(cleanup_error) = self.cleanup_chroot(&chroot_root).await
        {
            tracing::warn!(
                error = %cleanup_error,
                path = %chroot_root.display(),
                "failed to clean a sandbox jail after launch failure"
            );
        }
        result
    }

    async fn launch_inner(
        &self,
        spec: SandboxSpec,
        id: SandboxId,
        restore: Option<RestoreAssets>,
        capture_template: bool,
    ) -> Result<SandboxHandle, RuntimeError> {
        spec.validate()
            .map_err(|error| RuntimeError::invalid(error.to_string()))?;
        let restore_snapshot = restore.is_some();
        let chroot_root = self.jail_root(&id)?;
        let kernel_path = chroot_root.join("kernel").join("vmlinux");
        let rootfs_path = chroot_root.join("images").join("rootfs.ext4");
        let run_path = chroot_root.join("run");
        let jail_snapshot_path = chroot_root.join("snapshot");
        fs::create_dir_all(kernel_path.parent().expect("kernel parent"))
            .await
            .map_err(|error| RuntimeError::internal(format!("create kernel dir: {error}")))?;
        fs::create_dir_all(rootfs_path.parent().expect("rootfs parent"))
            .await
            .map_err(|error| RuntimeError::internal(format!("create image dir: {error}")))?;
        fs::create_dir_all(&run_path)
            .await
            .map_err(|error| RuntimeError::internal(format!("create run dir: {error}")))?;
        fs::create_dir_all(&jail_snapshot_path)
            .await
            .map_err(|error| RuntimeError::internal(format!("create snapshot dir: {error}")))?;
        clone_readonly_asset(&self.config.kernel_image, &kernel_path)
            .await
            .map_err(|error| RuntimeError::internal(format!("clone kernel: {error}")))?;
        let rootfs_source = restore.as_ref().map_or_else(
            || self.config.rootfs_template.clone(),
            |assets| assets.rootfs_path.clone(),
        );
        clone_rootfs(&rootfs_source, &rootfs_path)
            .await
            .map_err(|error| RuntimeError::internal(error.to_string()))?;
        if let Some(assets) = &restore {
            clone_readonly_asset(
                &assets.vmstate_path,
                &jail_snapshot_path.join("vmstate"),
            )
            .await
            .map_err(|error| RuntimeError::internal(format!("clone snapshot state: {error}")))?;
            clone_readonly_asset(
                &assets.memory_path,
                &jail_snapshot_path.join("memory"),
            )
            .await
            .map_err(|error| RuntimeError::internal(format!("clone snapshot memory: {error}")))?;
        }

        let permission_status = Command::new("chmod")
            .arg("0600")
            .arg(&rootfs_path)
            .status()
            .await
            .map_err(|error| RuntimeError::internal(format!("start chmod: {error}")))?;
        if !permission_status.success() {
            return Err(RuntimeError::internal("chmod sandbox rootfs failed"));
        }
        let owner = format!("{}:{}", self.config.jail_uid, self.config.jail_gid);
        let ownership_status = Command::new("chown")
            .arg(&owner)
            .arg(&rootfs_path)
            .arg(&run_path)
            .arg(&jail_snapshot_path)
            .status()
            .await
            .map_err(|error| RuntimeError::internal(format!("start chown: {error}")))?;
        if !ownership_status.success() {
            return Err(RuntimeError::internal("chown jail assets failed"));
        }

        let network = self.network.create(&id, spec.network).await?;
        let memory_limit = format!(
            "memory.max={}",
            u64::from(spec.memory_mb + 128) * 1024 * 1024
        );
        let cpu_limit = format!("cpu.max={} 100000", u64::from(spec.cpu_count) * 100000);
        let mut command = Command::new(&self.config.jailer_binary);
        command
            .arg("--id")
            .arg(id.to_string())
            .arg("--exec-file")
            .arg(&self.config.firecracker_binary)
            .arg("--uid")
            .arg(self.config.jail_uid.to_string())
            .arg("--gid")
            .arg(self.config.jail_gid.to_string())
            .arg("--chroot-base-dir")
            .arg(&self.config.chroot_base)
            .arg("--cgroup-version")
            .arg("2")
            .arg("--parent-cgroup")
            .arg("ferrobox")
            .arg("--cgroup")
            .arg(memory_limit)
            .arg("--cgroup")
            .arg("pids.max=512")
            .arg("--cgroup")
            .arg(cpu_limit)
            .arg("--resource-limit")
            .arg("no-file=1024")
            .arg("--new-pid-ns");
        if let Some(lease) = &network {
            command.arg("--netns").arg(&lease.namespace_path);
        }
        command
            .args(["--", "--api-sock", "/run/firecracker.socket"])
            .stdout(Stdio::null());
        let mut child = match command
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                if let Some(lease) = &network {
                    let _ = self.network.delete(lease).await;
                }
                return Err(RuntimeError::internal(format!("start jailer: {error}")));
            }
        };

        let api_socket = run_path.join("firecracker.socket");
        if let Err(error) = Self::wait_for_socket(&api_socket, self.config.api_timeout).await {
            let _ = child.kill().await;
            if let Some(lease) = &network {
                let _ = self.network.delete(lease).await;
            }
            return Err(error);
        }
        let api = FirecrackerClient::new(api_socket, self.config.api_timeout);
        let start_result = if restore_snapshot {
            self.load_snapshot(&api).await
        } else {
            self.configure_and_start(&api, &spec, network.as_ref())
                .await
        };
        if let Err(error) = start_result {
            let _ = child.kill().await;
            if let Some(lease) = &network {
                let _ = self.network.delete(lease).await;
            }
            return Err(error);
        }
        let connector = GuestConnector::new(
            run_path.join("guest.vsock"),
            self.config.guest_port,
            self.config.api_timeout,
        );
        if capture_template {
            if let Err(error) = self.wait_for_guest(&connector).await {
                let _ = child.kill().await;
                if let Some(lease) = &network {
                    let _ = self.network.delete(lease).await;
                }
                return Err(error);
            }
            if let Err(error) = self
                .capture_snapshot(&api, &rootfs_path, &jail_snapshot_path)
                .await
            {
                let _ = child.kill().await;
                if let Some(lease) = &network {
                    let _ = self.network.delete(lease).await;
                }
                return Err(error);
            }
        }
        let guest_token = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let initialized = if let Some(previous_token) = restore
            .as_ref()
            .and_then(|assets| assets.captured_guest_token.as_deref())
        {
            self.rekey_guest(
                &id,
                &connector,
                previous_token,
                &guest_token,
                network.as_ref(),
            )
            .await
        } else {
            self.initialize_guest(&id, &connector, &guest_token, network.as_ref())
                .await
        };
        let guest_client = match initialized {
            Ok(client) => client,
            Err(error) => {
                let _ = child.kill().await;
                if let Some(lease) = &network {
                    let _ = self.network.delete(lease).await;
                }
                return Err(error);
            }
        };
        self.sandboxes.write().await.insert(
            id.clone(),
            Arc::new(Mutex::new(FirecrackerSandbox {
                state: SandboxState::Running,
                spec,
                child,
                api,
                guest_client,
                guest_token,
                chroot_root,
                network,
            })),
        );
        Ok(SandboxHandle {
            sandbox_id: id,
            node_id: self.config.node_id.clone(),
            state: SandboxState::Running,
        })
    }
}

#[async_trait]
impl SandboxRuntime for FirecrackerRuntime {
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, RuntimeError> {
        spec.validate()
            .map_err(|error| RuntimeError::invalid(error.to_string()))?;
        if spec.template_id == "python"
            && spec.cpu_count == 1
            && spec.memory_mb == 512
            && spec.network == ferrobox_core::NetworkMode::Disabled
            && let Some(handle) = self.ready_pool.lock().await.pop()
        {
            return Ok(handle);
        }
        self.create_fresh(spec).await
    }

    async fn execute(
        &self,
        sandbox_id: &SandboxId,
        request: ExecRequest,
    ) -> Result<ExecResult, RuntimeError> {
        self.execute_measured(sandbox_id, request)
            .await
            .map(|(result, _)| result)
    }

    async fn signal(
        &self,
        sandbox_id: &SandboxId,
        request: SignalRequest,
    ) -> Result<SignalResult, RuntimeError> {
        let (mut client, token_value) = self.guest_client(sandbox_id).await?;
        let response = client
            .signal_process(Request::new(GuestSignalRequest {
                auth: Some(Auth { token: token_value }),
                request_id: uuid::Uuid::new_v4().to_string(),
                process_id: request.process_id.to_string(),
                signal: request.signal,
            }))
            .await
            .map_err(guest_error)?
            .into_inner();
        Ok(SignalResult {
            delivered: response.delivered,
        })
    }

    async fn write(
        &self,
        sandbox_id: &SandboxId,
        request: WriteFileRequest,
    ) -> Result<WriteFileResult, RuntimeError> {
        let (mut client, token_value) = self.guest_client(sandbox_id).await?;
        let response = client
            .write_file(Request::new(GuestWriteRequest {
                auth: Some(Auth { token: token_value }),
                request_id: uuid::Uuid::new_v4().to_string(),
                path: request.path.to_string(),
                data: request.data,
                overwrite: request.overwrite,
                mode: request.mode,
            }))
            .await
            .map_err(guest_error)?
            .into_inner();
        Ok(WriteFileResult {
            bytes_written: response.bytes_written,
        })
    }

    async fn read(
        &self,
        sandbox_id: &SandboxId,
        request: ReadFileRequest,
    ) -> Result<ReadFileResult, RuntimeError> {
        let (mut client, token_value) = self.guest_client(sandbox_id).await?;
        let response = client
            .read_file(Request::new(GuestReadRequest {
                auth: Some(Auth { token: token_value }),
                request_id: uuid::Uuid::new_v4().to_string(),
                path: request.path.to_string(),
                offset: request.offset,
                max_bytes: request.max_bytes,
            }))
            .await
            .map_err(guest_error)?;
        let mut stream = response.into_inner();
        let mut data = Vec::new();
        let mut eof = false;
        while let Some(chunk) = stream.message().await.map_err(guest_error)? {
            data.extend(chunk.data);
            eof = chunk.eof;
        }
        Ok(ReadFileResult { data, eof })
    }

    async fn list(
        &self,
        sandbox_id: &SandboxId,
        request: ListDirectoryRequest,
    ) -> Result<ListDirectoryResult, RuntimeError> {
        let (mut client, token_value) = self.guest_client(sandbox_id).await?;
        let response = client
            .list_directory(Request::new(GuestListRequest {
                auth: Some(Auth { token: token_value }),
                request_id: uuid::Uuid::new_v4().to_string(),
                path: request.path.to_string(),
            }))
            .await
            .map_err(guest_error)?
            .into_inner();
        let entries = response
            .entries
            .into_iter()
            .map(|entry| {
                let kind = match guest::FileKind::try_from(entry.kind) {
                    Ok(guest::FileKind::File) => FileKind::File,
                    Ok(guest::FileKind::Directory) => FileKind::Directory,
                    _ => return Err(RuntimeError::internal("guest returned invalid file kind")),
                };
                Ok(DirectoryEntry {
                    name: entry.name,
                    path: SandboxPath::new(entry.path)
                        .map_err(|error| RuntimeError::internal(error.to_string()))?,
                    kind,
                    size_bytes: entry.size_bytes,
                    modified_unix_millis: entry.modified_unix_millis,
                    mode: entry.mode,
                })
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        Ok(ListDirectoryResult { entries })
    }

    async fn pause(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError> {
        let record = self.record(sandbox_id).await?;
        let mut record = record.lock().await;
        if record.state != SandboxState::Running {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "sandbox is not running",
            ));
        }
        record
            .api
            .patch(
                "/vm",
                &VmState {
                    state: VmStateValue::Paused,
                },
            )
            .await
            .map_err(fc_error)?;
        record.state = SandboxState::Paused;
        Ok(())
    }

    async fn resume(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError> {
        let record = self.record(sandbox_id).await?;
        let mut record = record.lock().await;
        if record.state != SandboxState::Paused {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "sandbox is not paused",
            ));
        }
        record
            .api
            .patch(
                "/vm",
                &VmState {
                    state: VmStateValue::Resumed,
                },
            )
            .await
            .map_err(fc_error)?;
        record.state = SandboxState::Running;
        Ok(())
    }

    async fn create_snapshot(
        &self,
        sandbox_id: &SandboxId,
        request: CreateSnapshotRequest,
    ) -> Result<SnapshotHandle, RuntimeError> {
        request
            .validate()
            .map_err(|error| RuntimeError::invalid(error.to_string()))?;
        let source = self.record(sandbox_id).await?;
        let snapshot_id = SnapshotId::new();
        let mut source = source.lock().await;
        let source_state = source.state;
        if !matches!(source_state, SandboxState::Running | SandboxState::Paused) {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "sandbox must be running or paused to create a snapshot",
            ));
        }
        let stem = format!("user-{snapshot_id}");
        let jail_snapshot_root = source.chroot_root.join("snapshot");
        let jail_vmstate = jail_snapshot_root.join(format!("{stem}.vmstate"));
        let jail_memory = jail_snapshot_root.join(format!("{stem}.memory"));
        let rootfs = source.chroot_root.join("images").join("rootfs.ext4");
        let mut paused_by_request = false;
        let capture = async {
            if source_state == SandboxState::Running {
                source
                    .api
                    .patch(
                        "/vm",
                        &VmState {
                            state: VmStateValue::Paused,
                        },
                    )
                    .await
                    .map_err(fc_error)?;
                source.state = SandboxState::Paused;
                paused_by_request = true;
            }
            source
                .api
                .put(
                    "/snapshot/create",
                    &SnapshotCreate {
                        snapshot_type: SnapshotType::Full,
                        snapshot_path: format!("/snapshot/{stem}.vmstate"),
                        mem_file_path: format!("/snapshot/{stem}.memory"),
                    },
                )
                .await
                .map_err(fc_error)?;
            self.snapshot_store
                .stage(SnapshotStageRequest {
                    snapshot_id: &snapshot_id,
                    source_sandbox_id: sandbox_id,
                    name: request.name,
                    source_state,
                    spec: &source.spec,
                    vmstate_path: &jail_vmstate,
                    memory_path: &jail_memory,
                    rootfs_path: &rootfs,
                    restore_token: &source.guest_token,
                })
                .await
        }
        .await;
        let _ = fs::remove_file(&jail_vmstate).await;
        let _ = fs::remove_file(&jail_memory).await;
        let resume = if paused_by_request {
            source
                .api
                .patch(
                    "/vm",
                    &VmState {
                        state: VmStateValue::Resumed,
                    },
                )
                .await
                .map_err(fc_error)
        } else {
            Ok(())
        };
        if resume.is_ok() {
            source.state = source_state;
        } else {
            source.state = SandboxState::Failed;
        }
        drop(source);
        let staged = match (capture, resume) {
            (Ok(staged), Ok(())) => staged,
            (Ok(staged), Err(error)) => {
                self.snapshot_store.discard(staged).await;
                return Err(error);
            }
            (Err(error), _) => return Err(error),
        };
        let artifact = self.snapshot_store.finalize(staged).await?;
        let handle = artifact.handle();
        self.snapshots
            .write()
            .await
            .insert(snapshot_id, Arc::new(Mutex::new(artifact)));
        Ok(handle)
    }

    async fn list_snapshots(
        &self,
        sandbox_id: &SandboxId,
    ) -> Result<Vec<SnapshotHandle>, RuntimeError> {
        self.record(sandbox_id).await?;
        let records: Vec<_> = self.snapshots.read().await.values().cloned().collect();
        let mut snapshots = Vec::new();
        for record in records {
            let record = record.lock().await;
            if record.source_sandbox_id() == sandbox_id {
                snapshots.push(record.handle());
            }
        }
        snapshots.sort_by(|left, right| {
            left.created_at_unix_ms
                .cmp(&right.created_at_unix_ms)
                .then_with(|| left.snapshot_id.to_string().cmp(&right.snapshot_id.to_string()))
        });
        Ok(snapshots)
    }

    async fn get_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<SnapshotHandle, RuntimeError> {
        let record = self.snapshot_record(snapshot_id).await?;
        let handle = record.lock().await.handle();
        Ok(handle)
    }

    async fn verify_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<SnapshotVerification, RuntimeError> {
        let record = self.snapshot_record(snapshot_id).await?;
        let record = record.lock().await;
        Ok(self.snapshot_store.verify(&record).await)
    }

    async fn restore_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<SandboxHandle, RuntimeError> {
        let mut restored = self.clone_snapshot(snapshot_id, 1).await?;
        restored
            .pop()
            .ok_or_else(|| RuntimeError::internal("snapshot restore produced no sandbox"))
    }

    async fn clone_snapshot(
        &self,
        snapshot_id: &SnapshotId,
        count: u8,
    ) -> Result<Vec<SandboxHandle>, RuntimeError> {
        if !(1..=32).contains(&count) {
            return Err(RuntimeError::invalid("clone count must be between 1 and 32"));
        }
        let record = self.snapshot_record(snapshot_id).await?;
        let record = record.lock().await;
        let verification = self.snapshot_store.verify(&record).await;
        if !verification.valid {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unavailable,
                verification
                    .failure
                    .unwrap_or_else(|| "snapshot integrity verification failed".to_owned()),
            ));
        }
        let captured_guest_token = self.snapshot_store.restore_token(&record).await?;
        let mut handles = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            match self
                .create_from_snapshot_artifact(&record, captured_guest_token.clone())
                .await
            {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    for handle in &handles {
                        let _ = <Self as SandboxRuntime>::delete(self, &handle.sandbox_id).await;
                    }
                    return Err(error);
                }
            }
        }
        Ok(handles)
    }

    async fn rollback_snapshot(
        &self,
        sandbox_id: &SandboxId,
        snapshot_id: &SnapshotId,
    ) -> Result<SandboxHandle, RuntimeError> {
        let source_record = self.record(sandbox_id).await?;
        let mut source = source_record.lock().await;
        if !matches!(source.state, SandboxState::Running | SandboxState::Paused) {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "sandbox must be running or paused to roll back",
            ));
        }

        let snapshot_record = self.snapshot_record(snapshot_id).await?;
        let snapshot = snapshot_record.lock().await;
        if snapshot.source_sandbox_id() != sandbox_id {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "snapshot belongs to another sandbox",
            ));
        }
        let verification = self.snapshot_store.verify(&snapshot).await;
        if !verification.valid {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unavailable,
                verification
                    .failure
                    .unwrap_or_else(|| "snapshot integrity verification failed".to_owned()),
            ));
        }
        let captured_guest_token = self.snapshot_store.restore_token(&snapshot).await?;
        let replacement_handle = self
            .create_from_snapshot_artifact(&snapshot, captured_guest_token)
            .await?;
        drop(snapshot);

        let replacement_id = replacement_handle.sandbox_id;
        let replacement_record = self.record(&replacement_id).await?;
        let mut replacement = replacement_record.lock().await;
        let replacement_token = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let previous_token = replacement.guest_token.clone();
        let network = GuestNetworkConfig::from_lease(replacement.network.as_ref());
        if let Err(error) = self
            .rekey_connected_guest(
                sandbox_id,
                &mut replacement.guest_client,
                &previous_token,
                &replacement_token,
                &network,
            )
            .await
        {
            drop(replacement);
            let _ = <Self as SandboxRuntime>::delete(self, &replacement_id).await;
            return Err(error);
        }
        replacement.guest_token = replacement_token;

        let swap_result = {
            let mut sandboxes = self.sandboxes.write().await;
            let source_matches = sandboxes
                .get(sandbox_id)
                .is_some_and(|current| Arc::ptr_eq(current, &source_record));
            let replacement_matches = sandboxes
                .get(&replacement_id)
                .is_some_and(|current| Arc::ptr_eq(current, &replacement_record));
            if !source_matches || !replacement_matches {
                Err(RuntimeError::new(
                    RuntimeErrorKind::Conflict,
                    "sandbox changed while rollback was preparing",
                ))
            } else {
                sandboxes.remove(sandbox_id);
                sandboxes.remove(&replacement_id);
                sandboxes.insert(sandbox_id.clone(), Arc::clone(&replacement_record));
                Ok(())
            }
        };
        if let Err(error) = swap_result {
            drop(replacement);
            let _ = <Self as SandboxRuntime>::delete(self, &replacement_id).await;
            return Err(error);
        }
        drop(replacement);

        if let Err(error) = self.terminate_record(&mut source).await {
            tracing::warn!(
                sandbox_id = %sandbox_id,
                error = %error,
                "replacement VM committed while old VM cleanup reported an error"
            );
        }
        Ok(SandboxHandle {
            sandbox_id: sandbox_id.clone(),
            node_id: self.config.node_id.clone(),
            state: SandboxState::Running,
        })
    }

    async fn delete_snapshot(&self, snapshot_id: &SnapshotId) -> Result<(), RuntimeError> {
        let record = self.snapshot_record(snapshot_id).await?;
        let record_guard = record.try_lock().map_err(|_| {
            RuntimeError::new(RuntimeErrorKind::Conflict, "snapshot is currently in use")
        })?;
        self.snapshot_store.delete(&record_guard).await?;
        drop(record_guard);
        self.snapshots.write().await.remove(snapshot_id);
        Ok(())
    }

    async fn delete(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError> {
        let record = self
            .sandboxes
            .write()
            .await
            .remove(sandbox_id)
            .ok_or_else(|| RuntimeError::not_found("sandbox does not exist"))?;
        let mut record = record.lock().await;
        self.terminate_record(&mut record).await
    }
}

fn guest_cid(spec: &SandboxSpec) -> u32 {
    3 + (u32::from(spec.cpu_count) << 8) + (spec.memory_mb % 127)
}

fn parse_vm_rss_kib(status: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

fn fc_error(error: crate::firecracker::FirecrackerError) -> RuntimeError {
    RuntimeError::new(RuntimeErrorKind::Unavailable, error.to_string())
}

fn guest_error(error: tonic::Status) -> RuntimeError {
    let kind = match error.code() {
        tonic::Code::NotFound => RuntimeErrorKind::NotFound,
        tonic::Code::InvalidArgument => RuntimeErrorKind::InvalidInput,
        tonic::Code::Unauthenticated | tonic::Code::PermissionDenied => {
            RuntimeErrorKind::Unauthorized
        }
        tonic::Code::ResourceExhausted => RuntimeErrorKind::ResourceExhausted,
        tonic::Code::DeadlineExceeded => RuntimeErrorKind::Timeout,
        _ => RuntimeErrorKind::Unavailable,
    };
    RuntimeError::new(kind, error.message())
}

#[cfg(test)]
mod tests {
    use super::parse_vm_rss_kib;

    #[test]
    fn parses_linux_process_rss() {
        assert_eq!(
            parse_vm_rss_kib("Name:\tfirecracker\nVmRSS:\t  12345 kB\n"),
            Some(12345)
        );
    }
}
