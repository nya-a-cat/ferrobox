use std::{
    collections::HashMap,
    io::{Read as _, Seek as _, Write as _},
    os::unix::process::ExitStatusExt as _,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use cap_std::{ambient_authority, fs::Dir};
use ferrobox_core::SandboxPath;
use ferrobox_protocol::guest::v1::{
    self as protocol, DirectoryEntry, FileChunk, HealthRequest, HealthResponse, InitRequest,
    InitResponse, ListDirectoryRequest, ListDirectoryResponse, ProcessError, ProcessEvent,
    ProcessExit, ReadFileRequest, SignalProcessRequest, SignalProcessResponse, StartProcessRequest,
    WriteFileRequest, WriteFileResponse, guest_service_server, process_event,
};
use futures::Stream;
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;
use tokio::{
    io::AsyncReadExt as _,
    process::Command,
    sync::{RwLock, mpsc},
    time::timeout,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

type RpcStream<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;
const FILE_CHUNK_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
struct InitState {
    sandbox_id: String,
    token_digest: [u8; 32],
    uid: u32,
    gid: u32,
    max_file_bytes: u64,
    guest_cgroup_available: bool,
}

pub struct GuestService {
    workspace: PathBuf,
    initialization: Arc<RwLock<Option<InitState>>>,
    processes: Arc<RwLock<HashMap<String, u32>>>,
}

impl GuestService {
    pub fn new(workspace: PathBuf) -> Result<Self, std::io::Error> {
        std::fs::create_dir_all(&workspace)?;
        Ok(Self {
            workspace,
            initialization: Arc::new(RwLock::new(None)),
            processes: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    async fn authorized(&self, auth: Option<&protocol::Auth>) -> Result<InitState, Status> {
        let state = self
            .initialization
            .read()
            .await
            .clone()
            .ok_or_else(|| Status::failed_precondition("guest has not been initialized"))?;
        let token = auth
            .map(|value| value.token.as_str())
            .ok_or_else(|| Status::unauthenticated("missing token"))?;
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if !bool::from(state.token_digest.ct_eq(&digest)) {
            return Err(Status::unauthenticated("invalid token"));
        }
        Ok(state)
    }

    fn relative_path(path: &str) -> Result<PathBuf, Status> {
        let path = SandboxPath::new(path.to_owned())
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        Ok(PathBuf::from(path.relative()))
    }

    fn verify_components(
        directory: &Dir,
        relative: &Path,
        allow_missing_leaf: bool,
    ) -> Result<(), Status> {
        let mut current = PathBuf::new();
        let count = relative.components().count();
        for (index, component) in relative.components().enumerate() {
            current.push(component.as_os_str());
            match directory.symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(Status::permission_denied("symbolic links are forbidden"));
                }
                Ok(metadata) if index + 1 != count && !metadata.is_dir() => {
                    return Err(Status::invalid_argument("parent is not a directory"));
                }
                Ok(_) => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && allow_missing_leaf
                        && index + 1 == count => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(Status::not_found("path does not exist"));
                }
                Err(error) => return Err(Status::internal(error.to_string())),
            }
        }
        Ok(())
    }

    fn open_workspace(path: &Path) -> Result<Dir, Status> {
        Dir::open_ambient_dir(path, ambient_authority())
            .map_err(|error| Status::internal(error.to_string()))
    }

    fn directory_relative_path(relative: &Path) -> &Path {
        if relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            relative
        }
    }
}

#[tonic::async_trait]
impl guest_service_server::GuestService for GuestService {
    type StartProcessStream = RpcStream<ProcessEvent>;
    type ReadFileStream = RpcStream<FileChunk>;

    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        let initialization = self.initialization.read().await;
        Ok(Response::new(HealthResponse {
            ready: true,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            sandbox_id: initialization
                .as_ref()
                .map_or_else(String::new, |state| state.sandbox_id.clone()),
        }))
    }

    async fn init(&self, request: Request<InitRequest>) -> Result<Response<InitResponse>, Status> {
        let request = request.into_inner();
        if request.sandbox_id.is_empty()
            || request.token.len() < 32
            || request.command_uid == 0
            || request.max_file_bytes == 0
            || request.max_processes == 0
        {
            return Err(Status::invalid_argument("invalid initialization"));
        }
        let candidate = InitState {
            sandbox_id: request.sandbox_id,
            token_digest: Sha256::digest(request.token.as_bytes()).into(),
            uid: request.command_uid,
            gid: request.command_gid,
            max_file_bytes: request.max_file_bytes,
            guest_cgroup_available: configure_pids_limit(request.max_processes).await?,
        };
        let mut state = self.initialization.write().await;
        if let Some(existing) = state.as_ref() {
            let replayed = existing.sandbox_id == candidate.sandbox_id
                && bool::from(existing.token_digest.ct_eq(&candidate.token_digest))
                && existing.uid == candidate.uid
                && existing.gid == candidate.gid;
            if replayed {
                return Ok(Response::new(InitResponse {
                    initialized: true,
                    replayed: true,
                }));
            }
            return Err(Status::failed_precondition(
                "guest already initialized with different identity",
            ));
        }
        configure_guest_network(
            &request.guest_ipv4,
            request.guest_prefix_length,
            &request.gateway_ipv4,
            &request.dns_ipv4,
        )
        .await?;
        *state = Some(candidate);
        Ok(Response::new(InitResponse {
            initialized: true,
            replayed: false,
        }))
    }

    async fn rekey(
        &self,
        request: Request<protocol::RekeyRequest>,
    ) -> Result<Response<protocol::RekeyResponse>, Status> {
        let request = request.into_inner();
        self.authorized(request.auth.as_ref()).await?;
        if request.sandbox_id.is_empty()
            || request.token.len() < 32
            || request.command_uid == 0
            || request.max_file_bytes == 0
            || request.max_processes == 0
        {
            return Err(Status::invalid_argument("invalid rekey request"));
        }
        if !request.guest_ipv4.is_empty() {
            run_guest_command(
                "ip",
                &["address", "flush", "dev", "eth0", "scope", "global"],
            )
            .await?;
        }
        configure_guest_network(
            &request.guest_ipv4,
            request.guest_prefix_length,
            &request.gateway_ipv4,
            &request.dns_ipv4,
        )
        .await?;
        let candidate = InitState {
            sandbox_id: request.sandbox_id,
            token_digest: Sha256::digest(request.token.as_bytes()).into(),
            uid: request.command_uid,
            gid: request.command_gid,
            max_file_bytes: request.max_file_bytes,
            guest_cgroup_available: configure_pids_limit(request.max_processes).await?,
        };
        *self.initialization.write().await = Some(candidate);
        Ok(Response::new(protocol::RekeyResponse { rekeyed: true }))
    }

    async fn start_process(
        &self,
        request: Request<StartProcessRequest>,
    ) -> Result<Response<Self::StartProcessStream>, Status> {
        let request = request.into_inner();
        let state = self.authorized(request.auth.as_ref()).await?;
        if request.argv.is_empty()
            || request.argv.len() > 256
            || request
                .argv
                .iter()
                .any(|value| value.as_bytes().contains(&0))
            || request.timeout_millis == 0
            || request.max_output_bytes == 0
        {
            return Err(Status::invalid_argument("invalid process request"));
        }
        let relative_cwd = Self::relative_path(&request.cwd)?;
        let workspace = Self::open_workspace(&self.workspace)?;
        Self::verify_components(&workspace, &relative_cwd, false)?;
        let cwd = self.workspace.join(relative_cwd);
        let process_id = uuid::Uuid::new_v4().to_string();
        let process_map = Arc::clone(&self.processes);
        let (sender, receiver) = mpsc::channel(32);
        tokio::spawn(async move {
            let mut command = Command::new(&request.argv[0]);
            command
                .args(&request.argv[1..])
                .current_dir(cwd)
                .envs(request.environment)
                .uid(state.uid)
                .gid(state.gid)
                .kill_on_drop(true)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    send_error(&sender, &process_id, "spawn", &error.to_string()).await;
                    return;
                }
            };
            let Some(pid) = child.id() else {
                send_error(&sender, &process_id, "spawn", "process has no pid").await;
                return;
            };
            if let Err(error) = assign_guest_cgroup(pid, state.guest_cgroup_available) {
                let _ = child.kill().await;
                send_error(&sender, &process_id, "cgroup", error.message()).await;
                return;
            }
            process_map.write().await.insert(process_id.clone(), pid);
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let used = Arc::new(AtomicU64::new(0));
            let limit_hit = Arc::new(AtomicBool::new(false));
            let stdout_task = tokio::spawn(read_output(
                stdout,
                true,
                process_id.clone(),
                sender.clone(),
                Arc::clone(&used),
                Arc::clone(&limit_hit),
                request.max_output_bytes,
                pid,
            ));
            let stderr_task = tokio::spawn(read_output(
                stderr,
                false,
                process_id.clone(),
                sender.clone(),
                Arc::clone(&used),
                Arc::clone(&limit_hit),
                request.max_output_bytes,
                pid,
            ));
            let waited = timeout(Duration::from_millis(request.timeout_millis), child.wait()).await;
            let (status, timed_out) = match waited {
                Ok(Ok(status)) => (Some(status), false),
                Ok(Err(error)) => {
                    send_error(&sender, &process_id, "wait", &error.to_string()).await;
                    (None, false)
                }
                Err(_) => {
                    let _ = child.kill().await;
                    let status = child.wait().await.ok();
                    (status, true)
                }
            };
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            process_map.write().await.remove(&process_id);
            if let Some(status) = status {
                let exceeded = limit_hit.load(Ordering::Relaxed);
                let exit = ProcessExit {
                    exit_code: status.code(),
                    signal: status.signal(),
                    timed_out,
                    output_limit_exceeded: exceeded,
                    stdout_truncated: exceeded,
                    stderr_truncated: exceeded,
                };
                let _ = sender
                    .send(Ok(ProcessEvent {
                        process_id,
                        event: Some(process_event::Event::Exit(exit)),
                    }))
                    .await;
            }
        });
        Ok(Response::new(Box::pin(ReceiverStream::new(receiver))))
    }

    async fn signal_process(
        &self,
        request: Request<SignalProcessRequest>,
    ) -> Result<Response<SignalProcessResponse>, Status> {
        let request = request.into_inner();
        self.authorized(request.auth.as_ref()).await?;
        let pid = self
            .processes
            .read()
            .await
            .get(&request.process_id)
            .copied()
            .ok_or_else(|| Status::not_found("process does not exist"))?;
        let signal = Signal::try_from(request.signal)
            .map_err(|_| Status::invalid_argument("invalid signal"))?;
        kill(Pid::from_raw(pid as i32), signal)
            .map_err(|error| Status::internal(error.to_string()))?;
        Ok(Response::new(SignalProcessResponse { delivered: true }))
    }

    async fn write_file(
        &self,
        request: Request<WriteFileRequest>,
    ) -> Result<Response<WriteFileResponse>, Status> {
        let request = request.into_inner();
        let state = self.authorized(request.auth.as_ref()).await?;
        if request.data.len() as u64 > state.max_file_bytes {
            return Err(Status::resource_exhausted("file exceeds upload limit"));
        }
        let relative = Self::relative_path(&request.path)?;
        let workspace_path = self.workspace.clone();
        let response = tokio::task::spawn_blocking(move || -> Result<WriteFileResponse, Status> {
            let directory = Self::open_workspace(&workspace_path)?;
            Self::verify_components(&directory, &relative, true)?;
            if let Some(parent) = relative.parent() {
                directory
                    .create_dir_all(parent)
                    .map_err(|error| Status::internal(error.to_string()))?;
            }
            let mut options = cap_std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(request.overwrite);
            if !request.overwrite {
                options.create_new(true);
            }
            let mut file = directory
                .open_with(&relative, &options)
                .map_err(map_file_error)?;
            file.write_all(&request.data)
                .map_err(|error| Status::internal(error.to_string()))?;
            file.flush()
                .map_err(|error| Status::internal(error.to_string()))?;
            Ok(WriteFileResponse {
                bytes_written: request.data.len() as u64,
            })
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))??;
        Ok(Response::new(response))
    }

    async fn read_file(
        &self,
        request: Request<ReadFileRequest>,
    ) -> Result<Response<Self::ReadFileStream>, Status> {
        let request = request.into_inner();
        let state = self.authorized(request.auth.as_ref()).await?;
        let relative = Self::relative_path(&request.path)?;
        let workspace_path = self.workspace.clone();
        let maximum = request.max_bytes.min(state.max_file_bytes);
        let chunks = tokio::task::spawn_blocking(move || {
            let directory = Self::open_workspace(&workspace_path)?;
            Self::verify_components(&directory, &relative, false)?;
            let metadata = directory.metadata(&relative).map_err(map_file_error)?;
            if !metadata.is_file() {
                return Err(Status::invalid_argument("path is not a regular file"));
            }
            let mut file = directory.open(&relative).map_err(map_file_error)?;
            file.seek(std::io::SeekFrom::Start(request.offset))
                .map_err(|error| Status::internal(error.to_string()))?;
            let mut remaining = maximum;
            let mut chunks = Vec::new();
            while remaining > 0 {
                let size = usize::try_from(remaining.min(FILE_CHUNK_BYTES))
                    .unwrap_or(FILE_CHUNK_BYTES as usize);
                let mut data = vec![0; size];
                let read = file
                    .read(&mut data)
                    .map_err(|error| Status::internal(error.to_string()))?;
                data.truncate(read);
                remaining = remaining.saturating_sub(read as u64);
                let eof = read == 0
                    || request.offset + maximum.saturating_sub(remaining) >= metadata.len();
                chunks.push(Ok(FileChunk { data, eof }));
                if eof {
                    break;
                }
            }
            if chunks.is_empty() {
                chunks.push(Ok(FileChunk {
                    data: Vec::new(),
                    eof: true,
                }));
            }
            Ok(chunks)
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))??;
        Ok(Response::new(Box::pin(tokio_stream::iter(chunks))))
    }

    async fn list_directory(
        &self,
        request: Request<ListDirectoryRequest>,
    ) -> Result<Response<ListDirectoryResponse>, Status> {
        let request = request.into_inner();
        self.authorized(request.auth.as_ref()).await?;
        let relative = Self::relative_path(&request.path)?;
        let workspace_path = self.workspace.clone();
        let base = request.path.trim_end_matches('/').to_owned();
        let entries = tokio::task::spawn_blocking(move || {
            let directory = Self::open_workspace(&workspace_path)?;
            Self::verify_components(&directory, &relative, false)?;
            let mut output = Vec::new();
            for entry in directory
                .read_dir(Self::directory_relative_path(&relative))
                .map_err(map_file_error)?
            {
                if output.len() >= 4096 {
                    return Err(Status::resource_exhausted("directory entry limit exceeded"));
                }
                let entry = entry.map_err(map_file_error)?;
                let metadata = entry.metadata().map_err(map_file_error)?;
                let kind = if metadata.is_file() {
                    protocol::FileKind::File
                } else if metadata.is_dir() {
                    protocol::FileKind::Directory
                } else {
                    continue;
                };
                let name = entry.file_name().to_string_lossy().into_owned();
                let modified_unix_millis = metadata
                    .modified()
                    .ok()
                    .map(cap_std::time::SystemTime::into_std)
                    .and_then(|value| value.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .and_then(|value| u64::try_from(value.as_millis()).ok());
                output.push(DirectoryEntry {
                    path: format!("{base}/{name}"),
                    name,
                    kind: kind as i32,
                    size_bytes: metadata.len(),
                    modified_unix_millis,
                    mode: None,
                });
            }
            output.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(output)
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))??;
        Ok(Response::new(ListDirectoryResponse { entries }))
    }
}

#[allow(clippy::too_many_arguments)]
async fn read_output<R>(
    stream: Option<R>,
    stdout: bool,
    process_id: String,
    sender: mpsc::Sender<Result<ProcessEvent, Status>>,
    used: Arc<AtomicU64>,
    limit_hit: Arc<AtomicBool>,
    maximum: u64,
    pid: u32,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(mut stream) = stream else {
        return;
    };
    let mut buffer = vec![0; 16 * 1024];
    loop {
        let read = match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let previous = used.fetch_add(read as u64, Ordering::Relaxed);
        if previous >= maximum {
            limit_hit.store(true, Ordering::Relaxed);
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
            break;
        }
        let allowed = usize::try_from((maximum - previous).min(read as u64)).unwrap_or(read);
        if allowed < read {
            limit_hit.store(true, Ordering::Relaxed);
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
        }
        let event = if stdout {
            process_event::Event::Stdout(buffer[..allowed].to_vec())
        } else {
            process_event::Event::Stderr(buffer[..allowed].to_vec())
        };
        if sender
            .send(Ok(ProcessEvent {
                process_id: process_id.clone(),
                event: Some(event),
            }))
            .await
            .is_err()
        {
            let _ = kill(Pid::from_raw(pid as i32), Signal::SIGKILL);
            break;
        }
        if allowed < read {
            break;
        }
    }
}

async fn send_error(
    sender: &mpsc::Sender<Result<ProcessEvent, Status>>,
    process_id: &str,
    code: &str,
    message: &str,
) {
    let _ = sender
        .send(Ok(ProcessEvent {
            process_id: process_id.to_owned(),
            event: Some(process_event::Event::Error(ProcessError {
                code: code.to_owned(),
                message: message.to_owned(),
            })),
        }))
        .await;
}

async fn configure_guest_network(
    address: &str,
    prefix_length: u32,
    gateway: &str,
    dns: &str,
) -> Result<(), Status> {
    if address.is_empty() {
        if prefix_length != 0 || !gateway.is_empty() || !dns.is_empty() {
            return Err(Status::invalid_argument(
                "incomplete guest network configuration",
            ));
        }
        return Ok(());
    }
    let address = address
        .parse::<std::net::Ipv4Addr>()
        .map_err(|_| Status::invalid_argument("invalid guest IPv4 address"))?;
    let gateway = gateway
        .parse::<std::net::Ipv4Addr>()
        .map_err(|_| Status::invalid_argument("invalid gateway IPv4 address"))?;
    let dns = dns
        .parse::<std::net::Ipv4Addr>()
        .map_err(|_| Status::invalid_argument("invalid DNS IPv4 address"))?;
    if !(1..=32).contains(&prefix_length) {
        return Err(Status::invalid_argument("invalid guest prefix length"));
    }
    let address_with_prefix = format!("{address}/{prefix_length}");
    let gateway = gateway.to_string();
    run_guest_command("ip", &["link", "set", "eth0", "up"]).await?;
    run_guest_command(
        "ip",
        &["address", "replace", &address_with_prefix, "dev", "eth0"],
    )
    .await?;
    run_guest_command("ip", &["route", "replace", "default", "via", &gateway]).await?;
    tokio::fs::write("/etc/resolv.conf", format!("nameserver {dns}\n"))
        .await
        .map_err(|error| Status::internal(format!("write resolv.conf: {error}")))
}

async fn run_guest_command(program: &str, arguments: &[&str]) -> Result<(), Status> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .await
        .map_err(|error| Status::internal(format!("start {program}: {error}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Status::internal(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}
async fn configure_pids_limit(maximum: u32) -> Result<bool, Status> {
    let root = Path::new("/sys/fs/cgroup/ferrobox-guest");
    if tokio::fs::create_dir_all(root).await.is_err() {
        tracing::warn!("guest cgroup is unavailable; relying on host pids limit");
        return Ok(false);
    }
    tokio::fs::write(root.join("pids.max"), maximum.to_string())
        .await
        .map_err(|error| Status::internal(format!("set pids.max: {error}")))?;
    Ok(true)
}

fn assign_guest_cgroup(pid: u32, available: bool) -> Result<(), Status> {
    if !available {
        return Ok(());
    }
    std::fs::write(
        "/sys/fs/cgroup/ferrobox-guest/cgroup.procs",
        pid.to_string(),
    )
    .map_err(|error| Status::internal(format!("assign process cgroup: {error}")))
}

fn map_file_error(error: std::io::Error) -> Status {
    match error.kind() {
        std::io::ErrorKind::NotFound => Status::not_found("path does not exist"),
        std::io::ErrorKind::AlreadyExists => Status::already_exists("path exists"),
        std::io::ErrorKind::PermissionDenied => Status::permission_denied("path is forbidden"),
        _ => Status::internal(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::GuestService;

    #[test]
    fn lists_the_workspace_root_through_the_capability_directory() {
        let workspace = tempfile::tempdir().expect("temporary workspace");
        std::fs::write(workspace.path().join("oci.txt"), b"ferrobox-oci\n").expect("write fixture");
        let directory = GuestService::open_workspace(workspace.path()).expect("open workspace");
        let relative = GuestService::relative_path("/home/sandbox").expect("workspace path");

        let names = directory
            .read_dir(GuestService::directory_relative_path(&relative))
            .expect("list workspace root")
            .map(|entry| {
                entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["oci.txt".to_owned()]);
    }
}
