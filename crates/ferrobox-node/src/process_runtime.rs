use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use ferrobox_core::{
    DirectoryEntry, ExecRequest, ExecResult, ExecTermination, FileKind, ListDirectoryRequest,
    ListDirectoryResult, OutputTruncation, ProcessId, ReadFileRequest, ReadFileResult, RuntimeError,
    RuntimeErrorKind, SandboxHandle, SandboxId, SandboxPath, SandboxRuntime, SandboxSpec,
    SandboxState, SignalRequest, SignalResult, WriteFileRequest, WriteFileResult, MAX_FILE_BYTES,
};
use tokio::{
    fs,
    process::Command,
    sync::RwLock,
    time::{timeout, Instant},
};

use crate::audit::{
    AuditAction, AuditEvent, AuditOutcome, AuditSink, TracingAuditSink,
};

#[derive(Clone)]
struct ProcessSandbox {
    workspace: PathBuf,
    state: SandboxState,
}

/// Development runtime that executes commands on the host.
///
/// This backend is intentionally explicit and provides no workload isolation.
pub struct ProcessRuntime {
    root: PathBuf,
    node_id: String,
    sandboxes: RwLock<HashMap<SandboxId, ProcessSandbox>>,
    audit: Arc<dyn AuditSink>,
}

impl ProcessRuntime {
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, RuntimeError> {
        let root = root.into();
        fs::create_dir_all(&root)
            .await
            .map_err(|error| RuntimeError::internal(format!("create process root: {error}")))?;
        Ok(Self {
            root,
            node_id: "process-dev".to_owned(),
            sandboxes: RwLock::new(HashMap::new()),
            audit: Arc::new(TracingAuditSink),
        })
    }

    #[must_use]
    pub fn with_audit_sink(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = audit;
        self
    }

    async fn sandbox(&self, id: &SandboxId) -> Result<ProcessSandbox, RuntimeError> {
        self.sandboxes
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| RuntimeError::not_found("sandbox does not exist"))
    }

    async fn record(
        &self,
        id: &SandboxId,
        action: AuditAction,
        outcome: AuditOutcome,
        detail: Option<String>,
    ) {
        self.audit
            .record(AuditEvent {
                sandbox_id: id.to_string(),
                action,
                outcome,
                occurred_at: SystemTime::now(),
                detail,
            })
            .await;
    }

    async fn safe_path(
        workspace: &Path,
        path: &SandboxPath,
        allow_missing_leaf: bool,
    ) -> Result<PathBuf, RuntimeError> {
        let relative = Path::new(path.relative());
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(RuntimeError::invalid("path escapes sandbox workspace"));
        }
        let target = workspace.join(relative);
        let mut current = workspace.to_path_buf();
        let components: Vec<_> = relative.components().collect();
        for (index, component) in components.iter().enumerate() {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(RuntimeError::invalid("symbolic links are not allowed"));
                }
                Ok(_) => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && allow_missing_leaf
                        && index + 1 == components.len() => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(RuntimeError::not_found("path does not exist"));
                }
                Err(error) => {
                    return Err(RuntimeError::internal(format!(
                        "inspect sandbox path: {error}"
                    )));
                }
            }
        }
        Ok(target)
    }
}

#[async_trait]
impl SandboxRuntime for ProcessRuntime {
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, RuntimeError> {
        spec.validate()
            .map_err(|error| RuntimeError::invalid(error.to_string()))?;
        let id = SandboxId::new();
        let workspace = self.root.join(id.to_string()).join("home").join("sandbox");
        fs::create_dir_all(&workspace)
            .await
            .map_err(|error| RuntimeError::internal(format!("create sandbox: {error}")))?;
        self.sandboxes.write().await.insert(
            id.clone(),
            ProcessSandbox {
                workspace,
                state: SandboxState::Running,
            },
        );
        self.record(&id, AuditAction::Create, AuditOutcome::Succeeded, None)
            .await;
        Ok(SandboxHandle {
            sandbox_id: id,
            node_id: self.node_id.clone(),
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
        let sandbox = self.sandbox(sandbox_id).await?;
        if sandbox.state != SandboxState::Running {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "sandbox is not running",
            ));
        }
        let cwd = Self::safe_path(&sandbox.workspace, &request.cwd, false).await?;
        let process_id = ProcessId::new(uuid::Uuid::new_v4().to_string())
            .map_err(|error| RuntimeError::internal(error.to_string()))?;
        self.record(
            sandbox_id,
            AuditAction::Execute,
            AuditOutcome::Started,
            Some(process_id.to_string()),
        )
        .await;

        let mut command = Command::new(&request.argv[0]);
        command
            .args(&request.argv[1..])
            .current_dir(cwd)
            .envs(&request.environment)
            .kill_on_drop(true);
        let started = Instant::now();
        let output = timeout(
            Duration::from_secs(request.timeout_seconds),
            command.output(),
        )
        .await;
        let result = match output {
            Err(_) => ExecResult {
                process_id,
                termination: ExecTermination::TimedOut,
                stdout: Vec::new(),
                stderr: Vec::new(),
                truncation: OutputTruncation::default(),
            },
            Ok(Err(error)) => {
                self.record(
                    sandbox_id,
                    AuditAction::Execute,
                    AuditOutcome::Failed,
                    Some(error.to_string()),
                )
                .await;
                return Err(RuntimeError::new(
                    RuntimeErrorKind::Unavailable,
                    format!("start command: {error}"),
                ));
            }
            Ok(Ok(output)) => {
                let limit = usize::try_from(request.max_output_bytes).unwrap_or(usize::MAX);
                let mut stdout = output.stdout;
                let mut stderr = output.stderr;
                let stdout_truncated = stdout.len() > limit;
                if stdout_truncated {
                    stdout.truncate(limit);
                }
                let remaining = limit.saturating_sub(stdout.len());
                let stderr_truncated = stderr.len() > remaining;
                if stderr_truncated {
                    stderr.truncate(remaining);
                }
                let termination = if stdout_truncated || stderr_truncated {
                    ExecTermination::OutputLimitExceeded
                } else if let Some(code) = output.status.code() {
                    ExecTermination::Exited { exit_code: code }
                } else {
                    ExecTermination::Signaled {
                        signal: exit_signal(&output.status),
                    }
                };
                ExecResult {
                    process_id,
                    termination,
                    stdout,
                    stderr,
                    truncation: OutputTruncation {
                        stdout: stdout_truncated,
                        stderr: stderr_truncated,
                    },
                }
            }
        };
        self.record(
            sandbox_id,
            AuditAction::Execute,
            AuditOutcome::Succeeded,
            Some(format!("elapsed_ms={}", started.elapsed().as_millis())),
        )
        .await;
        Ok(result)
    }

    async fn signal(
        &self,
        sandbox_id: &SandboxId,
        _request: SignalRequest,
    ) -> Result<SignalResult, RuntimeError> {
        self.sandbox(sandbox_id).await?;
        Err(RuntimeError::new(
            RuntimeErrorKind::Unsupported,
            "process runtime does not retain completed process handles",
        ))
    }

    async fn write(
        &self,
        sandbox_id: &SandboxId,
        request: WriteFileRequest,
    ) -> Result<WriteFileResult, RuntimeError> {
        if request.data.len() as u64 > MAX_FILE_BYTES {
            return Err(RuntimeError::new(
                RuntimeErrorKind::ResourceExhausted,
                "file exceeds upload limit",
            ));
        }
        let sandbox = self.sandbox(sandbox_id).await?;
        let target = Self::safe_path(&sandbox.workspace, &request.path, true).await?;
        let parent = target
            .parent()
            .ok_or_else(|| RuntimeError::invalid("file has no parent"))?;
        fs::create_dir_all(parent)
            .await
            .map_err(|error| RuntimeError::internal(format!("create parent: {error}")))?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(request.overwrite);
        if !request.overwrite {
            options.create_new(true);
        }
        let mut file = options.open(&target).await.map_err(|error| {
            let kind = if error.kind() == std::io::ErrorKind::AlreadyExists {
                RuntimeErrorKind::Conflict
            } else {
                RuntimeErrorKind::Internal
            };
            RuntimeError::new(kind, format!("open file: {error}"))
        })?;
        use tokio::io::AsyncWriteExt as _;
        file.write_all(&request.data)
            .await
            .map_err(|error| RuntimeError::internal(format!("write file: {error}")))?;
        file.flush()
            .await
            .map_err(|error| RuntimeError::internal(format!("flush file: {error}")))?;
        self.record(
            sandbox_id,
            AuditAction::WriteFile,
            AuditOutcome::Succeeded,
            Some(format!("bytes={}", request.data.len())),
        )
        .await;
        Ok(WriteFileResult {
            bytes_written: request.data.len() as u64,
        })
    }

    async fn read(
        &self,
        sandbox_id: &SandboxId,
        request: ReadFileRequest,
    ) -> Result<ReadFileResult, RuntimeError> {
        let sandbox = self.sandbox(sandbox_id).await?;
        let target = Self::safe_path(&sandbox.workspace, &request.path, false).await?;
        let metadata = fs::metadata(&target)
            .await
            .map_err(|error| RuntimeError::not_found(format!("read metadata: {error}")))?;
        if !metadata.is_file() {
            return Err(RuntimeError::invalid("path is not a regular file"));
        }
        let maximum = request.max_bytes.min(MAX_FILE_BYTES);
        use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};
        let mut file = fs::File::open(target)
            .await
            .map_err(|error| RuntimeError::not_found(format!("open file: {error}")))?;
        file.seek(std::io::SeekFrom::Start(request.offset))
            .await
            .map_err(|error| RuntimeError::internal(format!("seek file: {error}")))?;
        let mut data = Vec::new();
        file.take(maximum)
            .read_to_end(&mut data)
            .await
            .map_err(|error| RuntimeError::internal(format!("read file: {error}")))?;
        let eof = request.offset.saturating_add(data.len() as u64) >= metadata.len();
        self.record(
            sandbox_id,
            AuditAction::ReadFile,
            AuditOutcome::Succeeded,
            Some(format!("bytes={}", data.len())),
        )
        .await;
        Ok(ReadFileResult { data, eof })
    }

    async fn list(
        &self,
        sandbox_id: &SandboxId,
        request: ListDirectoryRequest,
    ) -> Result<ListDirectoryResult, RuntimeError> {
        let sandbox = self.sandbox(sandbox_id).await?;
        let target = Self::safe_path(&sandbox.workspace, &request.path, false).await?;
        let mut directory = fs::read_dir(target)
            .await
            .map_err(|error| RuntimeError::not_found(format!("open directory: {error}")))?;
        let mut entries = Vec::new();
        while let Some(entry) = directory
            .next_entry()
            .await
            .map_err(|error| RuntimeError::internal(format!("read directory: {error}")))?
        {
            if entries.len() >= 4096 {
                return Err(RuntimeError::new(
                    RuntimeErrorKind::ResourceExhausted,
                    "directory entry limit exceeded",
                ));
            }
            let metadata = entry
                .metadata()
                .await
                .map_err(|error| RuntimeError::internal(format!("read metadata: {error}")))?;
            let kind = if metadata.is_file() {
                FileKind::File
            } else if metadata.is_dir() {
                FileKind::Directory
            } else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let base = request.path.as_str().trim_end_matches('/');
            let path = SandboxPath::new(format!("{base}/{name}"))
                .map_err(|error| RuntimeError::internal(error.to_string()))?;
            let modified_unix_millis = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(SystemTime::UNIX_EPOCH).ok())
                .and_then(|value| u64::try_from(value.as_millis()).ok());
            entries.push(DirectoryEntry {
                name,
                path,
                kind,
                size_bytes: metadata.len(),
                modified_unix_millis,
                mode: file_mode(&metadata),
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(ListDirectoryResult { entries })
    }

    async fn pause(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError> {
        let mut sandboxes = self.sandboxes.write().await;
        let sandbox = sandboxes
            .get_mut(sandbox_id)
            .ok_or_else(|| RuntimeError::not_found("sandbox does not exist"))?;
        if sandbox.state != SandboxState::Running {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "sandbox is not running",
            ));
        }
        sandbox.state = SandboxState::Paused;
        Ok(())
    }

    async fn resume(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError> {
        let mut sandboxes = self.sandboxes.write().await;
        let sandbox = sandboxes
            .get_mut(sandbox_id)
            .ok_or_else(|| RuntimeError::not_found("sandbox does not exist"))?;
        if sandbox.state != SandboxState::Paused {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "sandbox is not paused",
            ));
        }
        sandbox.state = SandboxState::Running;
        Ok(())
    }

    async fn delete(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError> {
        let sandbox = self
            .sandboxes
            .write()
            .await
            .remove(sandbox_id)
            .ok_or_else(|| RuntimeError::not_found("sandbox does not exist"))?;
        let sandbox_root = sandbox
            .workspace
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| RuntimeError::internal("invalid sandbox root"))?;
        if !sandbox_root.starts_with(&self.root) || sandbox_root == self.root {
            return Err(RuntimeError::internal("refusing unsafe cleanup target"));
        }
        fs::remove_dir_all(sandbox_root)
            .await
            .map_err(|error| RuntimeError::internal(format!("remove sandbox: {error}")))?;
        self.record(
            sandbox_id,
            AuditAction::Delete,
            AuditOutcome::Succeeded,
            None,
        )
        .await;
        Ok(())
    }
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt as _;
    status.signal().unwrap_or_default()
}

#[cfg(not(unix))]
fn exit_signal(_status: &std::process::ExitStatus) -> i32 {
    0
}

#[cfg(unix)]
fn file_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn file_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}
