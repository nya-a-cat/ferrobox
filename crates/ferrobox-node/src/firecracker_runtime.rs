use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use ferrobox_core::{
    DirectoryEntry, ExecRequest, ExecResult, ExecTermination, FileKind, ListDirectoryRequest,
    ListDirectoryResult, OutputTruncation, ProcessId, ReadFileRequest, ReadFileResult,
    RuntimeError, RuntimeErrorKind, SandboxHandle, SandboxId, SandboxPath, SandboxRuntime,
    SandboxSpec, SandboxState, SignalRequest, SignalResult, WriteFileRequest, WriteFileResult,
};
use ferrobox_protocol::guest::v1::{
    self as guest, Auth, HealthRequest, InitRequest, ListDirectoryRequest as GuestListRequest,
    ReadFileRequest as GuestReadRequest, SignalProcessRequest as GuestSignalRequest,
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
    sandboxes: RwLock<HashMap<SandboxId, Arc<Mutex<FirecrackerSandbox>>>>,
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
        Ok(Self {
            config,
            network: NetworkManager,
            sandboxes: RwLock::new(HashMap::new()),
        })
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

    async fn wait_for_socket(path: &Path, limit: Duration) -> Result<(), RuntimeError> {
        let started = Instant::now();
        while started.elapsed() < limit {
            if fs::metadata(path).await.is_ok() {
                return Ok(());
            }
            sleep(Duration::from_millis(25)).await;
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
            sleep(Duration::from_millis(100)).await;
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
            sleep(Duration::from_millis(100)).await;
        }
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

#[async_trait]
impl SandboxRuntime for FirecrackerRuntime {
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, RuntimeError> {
        spec.validate()
            .map_err(|error| RuntimeError::invalid(error.to_string()))?;
        let snapshot_compatible = spec.cpu_count == 1
            && spec.memory_mb == 512
            && spec.network == ferrobox_core::NetworkMode::Disabled;
        let restore_snapshot = snapshot_compatible && self.snapshot_available().await;
        let id = SandboxId::new();
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
        let rootfs_source = if restore_snapshot {
            self.config
                .snapshot_root
                .as_ref()
                .expect("available snapshot has a root")
                .join("rootfs.ext4")
        } else {
            self.config.rootfs_template.clone()
        };
        clone_rootfs(&rootfs_source, &rootfs_path)
            .await
            .map_err(|error| RuntimeError::internal(error.to_string()))?;
        if restore_snapshot {
            let snapshot_root = self
                .config
                .snapshot_root
                .as_ref()
                .expect("available snapshot has a root");
            clone_readonly_asset(
                &snapshot_root.join("vmstate"),
                &jail_snapshot_path.join("vmstate"),
            )
            .await
            .map_err(|error| RuntimeError::internal(format!("clone snapshot state: {error}")))?;
            clone_readonly_asset(
                &snapshot_root.join("memory"),
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
        let mut child = command
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| RuntimeError::internal(format!("start jailer: {error}")))?;

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
        if snapshot_compatible && self.config.snapshot_root.is_some() && !restore_snapshot {
            if let Err(error) = self.wait_for_guest(&connector).await {
                let _ = child.kill().await;
                return Err(error);
            }
            if let Err(error) = self
                .capture_snapshot(&api, &rootfs_path, &jail_snapshot_path)
                .await
            {
                let _ = child.kill().await;
                return Err(error);
            }
        }
        let guest_token = format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let guest_client = match self
            .initialize_guest(&id, &connector, &guest_token, network.as_ref())
            .await
        {
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

    async fn execute(
        &self,
        sandbox_id: &SandboxId,
        request: ExecRequest,
    ) -> Result<ExecResult, RuntimeError> {
        request
            .validate()
            .map_err(|error| RuntimeError::invalid(error.to_string()))?;
        let (mut client, token_value) = self.guest_client(sandbox_id).await?;
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
        Ok(ExecResult {
            process_id: process_id
                .ok_or_else(|| RuntimeError::internal("guest omitted process id"))?,
            termination: termination
                .ok_or_else(|| RuntimeError::internal("guest omitted process exit"))?,
            stdout,
            stderr,
            truncation,
        })
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

    async fn delete(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError> {
        let record = self
            .sandboxes
            .write()
            .await
            .remove(sandbox_id)
            .ok_or_else(|| RuntimeError::not_found("sandbox does not exist"))?;
        let mut record = record.lock().await;
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
        if timeout(Duration::from_secs(2), record.child.wait())
            .await
            .is_err()
        {
            let _ = record.child.kill().await;
            let _ = record.child.wait().await;
        }
        if let Some(lease) = &record.network {
            let _ = self.network.delete(lease).await;
        }
        let chroot_root = record.chroot_root.clone();
        drop(record);
        if !chroot_root.starts_with(&self.config.chroot_base)
            || chroot_root == self.config.chroot_base
        {
            return Err(RuntimeError::internal("refusing unsafe chroot cleanup"));
        }
        if fs::metadata(&chroot_root).await.is_ok() {
            fs::remove_dir_all(&chroot_root)
                .await
                .map_err(|error| RuntimeError::internal(format!("remove jail: {error}")))?;
        }
        Ok(())
    }
}

fn guest_cid(spec: &SandboxSpec) -> u32 {
    3 + (u32::from(spec.cpu_count) << 8) + (spec.memory_mb % 127)
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
