//! Domain types and runtime contract shared by the Ferrobox control plane.

use std::{collections::BTreeMap, fmt};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const MAX_CPU_COUNT: u8 = 32;
pub const MIN_MEMORY_MB: u32 = 128;
pub const MAX_MEMORY_MB: u32 = 32 * 1024;
pub const MAX_TIMEOUT_SECONDS: u64 = 86_400;
pub const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_SNAPSHOT_NAME_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SandboxId(Uuid);

impl SandboxId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    #[must_use]
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for SandboxId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SandboxId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::str::FromStr for SandboxId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SnapshotId(Uuid);

impl SnapshotId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for SnapshotId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::str::FromStr for SnapshotId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    #[default]
    Disabled,
    Internet,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SandboxSpec {
    pub template_id: String,
    pub cpu_count: u8,
    pub memory_mb: u32,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub network: NetworkMode,
}

impl SandboxSpec {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.template_id.is_empty()
            || self.template_id.len() > 64
            || !self
                .template_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(ValidationError::InvalidTemplate);
        }
        if !(1..=MAX_CPU_COUNT).contains(&self.cpu_count) {
            return Err(ValidationError::CpuCount);
        }
        if !(MIN_MEMORY_MB..=MAX_MEMORY_MB).contains(&self.memory_mb) {
            return Err(ValidationError::Memory);
        }
        if !(1..=MAX_TIMEOUT_SECONDS).contains(&self.timeout_seconds) {
            return Err(ValidationError::Timeout);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxState {
    Creating,
    Running,
    Pausing,
    Paused,
    Resuming,
    Deleting,
    Deleted,
    Failed,
}

impl SandboxState {
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use SandboxState::{
            Creating, Deleted, Deleting, Failed, Paused, Pausing, Resuming, Running,
        };
        matches!(
            (self, next),
            (Creating, Running | Failed | Deleting)
                | (Running, Pausing | Deleting | Failed)
                | (Pausing, Paused | Failed | Deleting)
                | (Paused, Resuming | Deleting | Failed)
                | (Resuming, Running | Failed | Deleting)
                | (Failed, Deleting)
                | (Deleting, Deleted | Failed)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SandboxHandle {
    pub sandbox_id: SandboxId,
    pub node_id: String,
    pub state: SandboxState,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateSnapshotRequest {
    pub name: Option<String>,
}

impl CreateSnapshotRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.as_ref().is_some_and(|name| {
            name.is_empty()
                || name.len() > MAX_SNAPSHOT_NAME_BYTES
                || name.trim() != name
                || name.chars().any(char::is_control)
        }) {
            return Err(ValidationError::SnapshotName);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotHandle {
    pub snapshot_id: SnapshotId,
    pub source_sandbox_id: SandboxId,
    pub name: Option<String>,
    pub created_at_unix_ms: u128,
    pub source_state: SandboxState,
    pub spec: SandboxSpec,
    pub size_bytes: u64,
    pub digest_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotVerification {
    pub snapshot_id: SnapshotId,
    pub valid: bool,
    pub checked_artifacts: u8,
    pub failure: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProcessId(String);

impl ProcessId {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 {
            return Err(ValidationError::ProcessId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProcessId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SandboxPath(String);

impl SandboxPath {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        let invalid_absolute = value.starts_with('/')
            && value != "/home/sandbox"
            && !value.starts_with("/home/sandbox/");
        if value.as_bytes().contains(&0)
            || value.split('/').any(|part| part == "..")
            || invalid_absolute
        {
            return Err(ValidationError::SandboxPath);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn workspace() -> Self {
        Self("/home/sandbox".to_owned())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn relative(&self) -> &str {
        self.0
            .strip_prefix("/home/sandbox/")
            .or_else(|| self.0.strip_prefix("/home/sandbox"))
            .unwrap_or(&self.0)
            .trim_start_matches('/')
    }
}

impl Default for SandboxPath {
    fn default() -> Self {
        Self::workspace()
    }
}

impl fmt::Display for SandboxPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecRequest {
    pub argv: Vec<String>,
    #[serde(default = "SandboxPath::workspace")]
    pub cwd: SandboxPath,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    pub timeout_seconds: u64,
    pub max_output_bytes: u64,
}

impl ExecRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.argv.is_empty()
            || self.argv.len() > 256
            || self
                .argv
                .iter()
                .any(|argument| argument.as_bytes().contains(&0) || argument.len() > 128 * 1024)
        {
            return Err(ValidationError::Argv);
        }
        if !(1..=MAX_TIMEOUT_SECONDS).contains(&self.timeout_seconds) {
            return Err(ValidationError::Timeout);
        }
        if !(1..=MAX_OUTPUT_BYTES).contains(&self.max_output_bytes) {
            return Err(ValidationError::OutputLimit);
        }
        if self.environment.len() > 256
            || self.environment.iter().any(|(key, value)| {
                key.is_empty()
                    || key.contains('=')
                    || key.as_bytes().contains(&0)
                    || value.as_bytes().contains(&0)
            })
        {
            return Err(ValidationError::Environment);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExecTermination {
    Exited { exit_code: i32 },
    Signaled { signal: i32 },
    TimedOut,
    OutputLimitExceeded,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct OutputTruncation {
    pub stdout: bool,
    pub stderr: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExecResult {
    pub process_id: ProcessId,
    pub termination: ExecTermination,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncation: OutputTruncation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignalRequest {
    pub process_id: ProcessId,
    pub signal: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SignalResult {
    pub delivered: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WriteFileRequest {
    pub path: SandboxPath,
    pub data: Vec<u8>,
    #[serde(default)]
    pub overwrite: bool,
    pub mode: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WriteFileResult {
    pub bytes_written: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadFileRequest {
    pub path: SandboxPath,
    #[serde(default)]
    pub offset: u64,
    pub max_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReadFileResult {
    pub data: Vec<u8>,
    pub eof: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListDirectoryRequest {
    pub path: SandboxPath,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListDirectoryResult {
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: SandboxPath,
    pub kind: FileKind,
    pub size_bytes: u64,
    pub modified_unix_millis: Option<u64>,
    pub mode: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    File,
    Directory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeErrorKind {
    NotFound,
    Conflict,
    InvalidInput,
    Unauthorized,
    Unsupported,
    Timeout,
    ResourceExhausted,
    Unavailable,
    Internal,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct RuntimeError {
    kind: RuntimeErrorKind,
    message: String,
}

impl RuntimeError {
    #[must_use]
    pub fn new(kind: RuntimeErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> RuntimeErrorKind {
        self.kind
    }

    #[must_use]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(RuntimeErrorKind::NotFound, message)
    }

    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(RuntimeErrorKind::InvalidInput, message)
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(RuntimeErrorKind::Internal, message)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ValidationError {
    #[error("template_id must contain 1-64 ASCII letters, digits, hyphens, or underscores")]
    InvalidTemplate,
    #[error("cpu_count is outside the supported range")]
    CpuCount,
    #[error("memory_mb is outside the supported range")]
    Memory,
    #[error("timeout_seconds is outside the supported range")]
    Timeout,
    #[error("argv is empty or invalid")]
    Argv,
    #[error("environment is invalid")]
    Environment,
    #[error("max_output_bytes is outside the supported range")]
    OutputLimit,
    #[error("sandbox path escapes /home/sandbox")]
    SandboxPath,
    #[error("process id is invalid")]
    ProcessId,
    #[error("snapshot name must be 1-128 bytes without surrounding whitespace or control characters")]
    SnapshotName,
}

#[async_trait]
pub trait SandboxRuntime: Send + Sync {
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxHandle, RuntimeError>;
    async fn execute(
        &self,
        sandbox_id: &SandboxId,
        request: ExecRequest,
    ) -> Result<ExecResult, RuntimeError>;
    async fn signal(
        &self,
        sandbox_id: &SandboxId,
        request: SignalRequest,
    ) -> Result<SignalResult, RuntimeError>;
    async fn write(
        &self,
        sandbox_id: &SandboxId,
        request: WriteFileRequest,
    ) -> Result<WriteFileResult, RuntimeError>;
    async fn read(
        &self,
        sandbox_id: &SandboxId,
        request: ReadFileRequest,
    ) -> Result<ReadFileResult, RuntimeError>;
    async fn list(
        &self,
        sandbox_id: &SandboxId,
        request: ListDirectoryRequest,
    ) -> Result<ListDirectoryResult, RuntimeError>;
    async fn pause(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError>;
    async fn resume(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError>;
    async fn create_snapshot(
        &self,
        sandbox_id: &SandboxId,
        request: CreateSnapshotRequest,
    ) -> Result<SnapshotHandle, RuntimeError>;
    async fn list_snapshots(
        &self,
        sandbox_id: &SandboxId,
    ) -> Result<Vec<SnapshotHandle>, RuntimeError>;
    async fn get_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<SnapshotHandle, RuntimeError>;
    async fn verify_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<SnapshotVerification, RuntimeError>;
    async fn restore_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<SandboxHandle, RuntimeError>;
    async fn clone_snapshot(
        &self,
        snapshot_id: &SnapshotId,
        count: u8,
    ) -> Result<Vec<SandboxHandle>, RuntimeError>;
    async fn rollback_snapshot(
        &self,
        sandbox_id: &SandboxId,
        snapshot_id: &SnapshotId,
    ) -> Result<SandboxHandle, RuntimeError>;
    async fn delete_snapshot(&self, snapshot_id: &SnapshotId) -> Result<(), RuntimeError>;
    async fn delete(&self, sandbox_id: &SandboxId) -> Result<(), RuntimeError>;
}

#[cfg(test)]
mod tests {
    use super::{
        CreateSnapshotRequest, ExecRequest, NetworkMode, SandboxPath, SandboxSpec, SandboxState,
    };

    #[test]
    fn validates_spec_and_argv_without_shell_parsing() {
        let spec = SandboxSpec {
            template_id: "python".to_owned(),
            cpu_count: 1,
            memory_mb: 512,
            timeout_seconds: 300,
            network: NetworkMode::Disabled,
        };
        assert!(spec.validate().is_ok());

        let request = ExecRequest {
            argv: vec!["printf".to_owned(), "$(whoami);".to_owned()],
            cwd: SandboxPath::workspace(),
            environment: Default::default(),
            timeout_seconds: 30,
            max_output_bytes: 1024,
        };
        assert!(request.validate().is_ok());
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(SandboxPath::new("../../etc/shadow").is_err());
        assert!(SandboxPath::new("/etc/shadow").is_err());
        assert!(SandboxPath::new("/home/sandbox/data.txt").is_ok());
    }

    #[test]
    fn state_machine_rejects_invalid_jump() {
        assert!(SandboxState::Creating.can_transition_to(SandboxState::Running));
        assert!(!SandboxState::Creating.can_transition_to(SandboxState::Paused));
        assert!(SandboxState::Failed.can_transition_to(SandboxState::Deleting));
    }

    #[test]
    fn validates_snapshot_name() {
        assert!(
            CreateSnapshotRequest {
                name: Some("checkpoint-1".to_owned())
            }
            .validate()
            .is_ok()
        );
        assert!(
            CreateSnapshotRequest {
                name: Some(" padded".to_owned())
            }
            .validate()
            .is_err()
        );
    }
}
