use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use ferrobox_core::{
    RuntimeError, RuntimeErrorKind, SandboxId, SandboxSpec, SandboxState, SnapshotHandle,
    SnapshotId, SnapshotVerification,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::{
    fs,
    io::{AsyncReadExt as _, AsyncWriteExt as _},
};

use crate::rootfs::clone_rootfs;

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const MANIFEST_NAME: &str = "manifest.json";
const VMSTATE_NAME: &str = "vmstate";
const MEMORY_NAME: &str = "memory";
const ROOTFS_NAME: &str = "rootfs.ext4";
const RESTORE_TOKEN_NAME: &str = "restore-token";
const ARTIFACT_NAMES: [&str; 3] = [VMSTATE_NAME, MEMORY_NAME, ROOTFS_NAME];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ArtifactDigest {
    size_bytes: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SnapshotManifest {
    schema_version: u32,
    pub snapshot_id: SnapshotId,
    pub source_sandbox_id: SandboxId,
    pub name: Option<String>,
    pub created_at_unix_ms: u128,
    pub source_state: SandboxState,
    pub spec: SandboxSpec,
    node_id: String,
    architecture: String,
    firecracker_version: String,
    kernel_sha256: String,
    artifacts: BTreeMap<String, ArtifactDigest>,
    restore_token_sha256: String,
    size_bytes: u64,
    digest_sha256: String,
}

impl SnapshotManifest {
    fn handle(&self) -> SnapshotHandle {
        SnapshotHandle {
            snapshot_id: self.snapshot_id.clone(),
            source_sandbox_id: self.source_sandbox_id.clone(),
            name: self.name.clone(),
            created_at_unix_ms: self.created_at_unix_ms,
            source_state: self.source_state,
            spec: self.spec.clone(),
            size_bytes: self.size_bytes,
            digest_sha256: self.digest_sha256.clone(),
        }
    }

    fn calculated_digest(&self) -> Result<String, RuntimeError> {
        let mut unsigned = self.clone();
        unsigned.digest_sha256.clear();
        let bytes = serde_json::to_vec(&unsigned)
            .map_err(|error| RuntimeError::internal(format!("serialize manifest: {error}")))?;
        Ok(sha256_bytes(&bytes))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SnapshotArtifact {
    root: PathBuf,
    manifest: SnapshotManifest,
}

impl SnapshotArtifact {
    pub(crate) fn handle(&self) -> SnapshotHandle {
        self.manifest.handle()
    }

    pub(crate) fn source_sandbox_id(&self) -> &SandboxId {
        &self.manifest.source_sandbox_id
    }

    pub(crate) fn spec(&self) -> &SandboxSpec {
        &self.manifest.spec
    }

    pub(crate) fn vmstate_path(&self) -> PathBuf {
        self.root.join(VMSTATE_NAME)
    }

    pub(crate) fn memory_path(&self) -> PathBuf {
        self.root.join(MEMORY_NAME)
    }

    pub(crate) fn rootfs_path(&self) -> PathBuf {
        self.root.join(ROOTFS_NAME)
    }
}

pub(crate) struct SnapshotStageRequest<'a> {
    pub snapshot_id: &'a SnapshotId,
    pub source_sandbox_id: &'a SandboxId,
    pub name: Option<String>,
    pub source_state: SandboxState,
    pub spec: &'a SandboxSpec,
    pub vmstate_path: &'a Path,
    pub memory_path: &'a Path,
    pub rootfs_path: &'a Path,
    pub restore_token: &'a str,
}

pub(crate) struct StagedSnapshot {
    partial_root: PathBuf,
    final_root: PathBuf,
    manifest: SnapshotManifest,
}

#[derive(Clone, Debug)]
pub(crate) struct SnapshotStore {
    root: PathBuf,
    node_id: String,
    firecracker_version: String,
    kernel_sha256: String,
}

impl SnapshotStore {
    pub(crate) async fn new(
        root: PathBuf,
        node_id: String,
        firecracker_version: &str,
        kernel_path: &Path,
    ) -> Result<Self, RuntimeError> {
        fs::create_dir_all(&root)
            .await
            .map_err(|error| RuntimeError::internal(format!("create snapshot store: {error}")))?;
        set_mode(&root, 0o700).await?;
        Ok(Self {
            root,
            node_id,
            firecracker_version: firecracker_version.to_owned(),
            kernel_sha256: sha256_file(kernel_path).await?,
        })
    }

    pub(crate) async fn stage(
        &self,
        request: SnapshotStageRequest<'_>,
    ) -> Result<StagedSnapshot, RuntimeError> {
        let partial_root = self
            .root
            .join(format!(".{}.partial", request.snapshot_id));
        let final_root = self.root.join(request.snapshot_id.to_string());
        if fs::metadata(&partial_root).await.is_ok() || fs::metadata(&final_root).await.is_ok() {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Conflict,
                "snapshot artifact path already exists",
            ));
        }
        fs::create_dir(&partial_root)
            .await
            .map_err(|error| RuntimeError::internal(format!("create snapshot stage: {error}")))?;
        set_mode(&partial_root, 0o700).await?;
        let staged = self.stage_inner(request, partial_root.clone(), final_root).await;
        if staged.is_err() {
            let _ = fs::remove_dir_all(&partial_root).await;
        }
        staged
    }

    async fn stage_inner(
        &self,
        request: SnapshotStageRequest<'_>,
        partial_root: PathBuf,
        final_root: PathBuf,
    ) -> Result<StagedSnapshot, RuntimeError> {
        for (source, name) in [
            (request.vmstate_path, VMSTATE_NAME),
            (request.memory_path, MEMORY_NAME),
            (request.rootfs_path, ROOTFS_NAME),
        ] {
            clone_rootfs(source, &partial_root.join(name))
                .await
                .map_err(|error| RuntimeError::internal(format!("stage snapshot {name}: {error}")))?;
            set_mode(&partial_root.join(name), 0o444).await?;
            sync_file(&partial_root.join(name)).await?;
        }
        let token_path = partial_root.join(RESTORE_TOKEN_NAME);
        let mut token_file = fs::File::create(&token_path)
            .await
            .map_err(|error| RuntimeError::internal(format!("create restore token: {error}")))?;
        token_file
            .write_all(request.restore_token.as_bytes())
            .await
            .map_err(|error| RuntimeError::internal(format!("write restore token: {error}")))?;
        token_file
            .sync_all()
            .await
            .map_err(|error| RuntimeError::internal(format!("sync restore token: {error}")))?;
        drop(token_file);
        set_mode(&token_path, 0o400).await?;
        Ok(StagedSnapshot {
            partial_root,
            final_root,
            manifest: SnapshotManifest {
                schema_version: SNAPSHOT_SCHEMA_VERSION,
                snapshot_id: request.snapshot_id.clone(),
                source_sandbox_id: request.source_sandbox_id.clone(),
                name: request.name,
                created_at_unix_ms: unix_millis(),
                source_state: request.source_state,
                spec: request.spec.clone(),
                node_id: self.node_id.clone(),
                architecture: std::env::consts::ARCH.to_owned(),
                firecracker_version: self.firecracker_version.clone(),
                kernel_sha256: self.kernel_sha256.clone(),
                artifacts: BTreeMap::new(),
                restore_token_sha256: sha256_bytes(request.restore_token.as_bytes()),
                size_bytes: 0,
                digest_sha256: String::new(),
            },
        })
    }

    pub(crate) async fn finalize(
        &self,
        mut staged: StagedSnapshot,
    ) -> Result<SnapshotArtifact, RuntimeError> {
        let finalized = self.finalize_inner(&mut staged).await;
        if finalized.is_err() {
            let _ = fs::remove_dir_all(&staged.partial_root).await;
        }
        finalized
    }

    pub(crate) async fn discard(&self, staged: StagedSnapshot) {
        let _ = fs::remove_dir_all(staged.partial_root).await;
    }

    async fn finalize_inner(
        &self,
        staged: &mut StagedSnapshot,
    ) -> Result<SnapshotArtifact, RuntimeError> {
        for name in ARTIFACT_NAMES {
            let path = staged.partial_root.join(name);
            let size_bytes = fs::metadata(&path)
                .await
                .map_err(|error| RuntimeError::internal(format!("stat snapshot {name}: {error}")))?
                .len();
            staged.manifest.size_bytes = staged.manifest.size_bytes.saturating_add(size_bytes);
            staged.manifest.artifacts.insert(
                name.to_owned(),
                ArtifactDigest {
                    size_bytes,
                    sha256: sha256_file(&path).await?,
                },
            );
        }
        staged.manifest.digest_sha256 = staged.manifest.calculated_digest()?;
        let manifest_bytes = serde_json::to_vec_pretty(&staged.manifest)
            .map_err(|error| RuntimeError::internal(format!("serialize manifest: {error}")))?;
        let manifest_path = staged.partial_root.join(MANIFEST_NAME);
        let mut manifest_file = fs::File::create(&manifest_path)
            .await
            .map_err(|error| RuntimeError::internal(format!("create manifest: {error}")))?;
        manifest_file
            .write_all(&manifest_bytes)
            .await
            .map_err(|error| RuntimeError::internal(format!("write manifest: {error}")))?;
        manifest_file
            .sync_all()
            .await
            .map_err(|error| RuntimeError::internal(format!("sync manifest: {error}")))?;
        drop(manifest_file);
        set_mode(&manifest_path, 0o444).await?;
        sync_directory(&staged.partial_root).await?;
        fs::rename(&staged.partial_root, &staged.final_root)
            .await
            .map_err(|error| RuntimeError::internal(format!("publish snapshot: {error}")))?;
        if let Err(error) = sync_directory(&self.root).await {
            let _ = fs::remove_dir_all(&staged.final_root).await;
            let _ = sync_directory(&self.root).await;
            return Err(error);
        }
        Ok(SnapshotArtifact {
            root: staged.final_root.clone(),
            manifest: staged.manifest.clone(),
        })
    }

    pub(crate) async fn verify(
        &self,
        artifact: &SnapshotArtifact,
    ) -> SnapshotVerification {
        match self.verified_manifest(artifact).await {
            Ok(_) => SnapshotVerification {
                snapshot_id: artifact.manifest.snapshot_id.clone(),
                valid: true,
                checked_artifacts: ARTIFACT_NAMES.len() as u8,
                failure: None,
            },
            Err(error) => SnapshotVerification {
                snapshot_id: artifact.manifest.snapshot_id.clone(),
                valid: false,
                checked_artifacts: 0,
                failure: Some(error.to_string()),
            },
        }
    }

    pub(crate) async fn verified_manifest(
        &self,
        artifact: &SnapshotArtifact,
    ) -> Result<SnapshotManifest, RuntimeError> {
        let bytes = fs::read(artifact.root.join(MANIFEST_NAME))
            .await
            .map_err(|error| integrity_error(format!("read manifest: {error}")))?;
        let manifest: SnapshotManifest = serde_json::from_slice(&bytes)
            .map_err(|error| integrity_error(format!("parse manifest: {error}")))?;
        if manifest.schema_version != SNAPSHOT_SCHEMA_VERSION
            || manifest != artifact.manifest
            || manifest.architecture != std::env::consts::ARCH
            || manifest.firecracker_version != self.firecracker_version
            || manifest.kernel_sha256 != self.kernel_sha256
            || manifest.digest_sha256 != manifest.calculated_digest()?
            || manifest.artifacts.len() != ARTIFACT_NAMES.len()
        {
            return Err(integrity_error("manifest identity or compatibility mismatch"));
        }
        let mut total_size = 0_u64;
        for name in ARTIFACT_NAMES {
            let expected = manifest
                .artifacts
                .get(name)
                .ok_or_else(|| integrity_error(format!("manifest omits {name}")))?;
            let path = artifact.root.join(name);
            let actual_size = fs::metadata(&path)
                .await
                .map_err(|error| integrity_error(format!("stat {name}: {error}")))?
                .len();
            if actual_size != expected.size_bytes || sha256_file(&path).await? != expected.sha256 {
                return Err(integrity_error(format!("{name} digest mismatch")));
            }
            total_size = total_size.saturating_add(actual_size);
        }
        if total_size != manifest.size_bytes {
            return Err(integrity_error("snapshot size mismatch"));
        }
        let restore_token = fs::read(artifact.root.join(RESTORE_TOKEN_NAME))
            .await
            .map_err(|error| integrity_error(format!("read restore token: {error}")))?;
        if sha256_bytes(&restore_token) != manifest.restore_token_sha256 {
            return Err(integrity_error("restore token digest mismatch"));
        }
        Ok(manifest)
    }

    pub(crate) async fn restore_token(
        &self,
        artifact: &SnapshotArtifact,
    ) -> Result<String, RuntimeError> {
        let token = fs::read_to_string(artifact.root.join(RESTORE_TOKEN_NAME))
            .await
            .map_err(|error| integrity_error(format!("read restore token: {error}")))?;
        if token.len() < 32 {
            return Err(integrity_error("restore token is invalid"));
        }
        if sha256_bytes(token.as_bytes()) != artifact.manifest.restore_token_sha256 {
            return Err(integrity_error("restore token digest mismatch"));
        }
        Ok(token)
    }

    pub(crate) async fn delete(&self, artifact: &SnapshotArtifact) -> Result<(), RuntimeError> {
        if !artifact.root.starts_with(&self.root) || artifact.root == self.root {
            return Err(RuntimeError::internal("refusing unsafe snapshot cleanup"));
        }
        fs::remove_dir_all(&artifact.root)
            .await
            .map_err(|error| RuntimeError::internal(format!("delete snapshot: {error}")))?;
        sync_directory(&self.root).await
    }
}

async fn sha256_file(path: &Path) -> Result<String, RuntimeError> {
    let mut file = fs::File::open(path)
        .await
        .map_err(|error| RuntimeError::internal(format!("open digest input: {error}")))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let length = file
            .read(&mut buffer)
            .await
            .map_err(|error| RuntimeError::internal(format!("read digest input: {error}")))?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{digest:x}"))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

async fn sync_file(path: &Path) -> Result<(), RuntimeError> {
    fs::File::open(path)
        .await
        .map_err(|error| RuntimeError::internal(format!("open snapshot file: {error}")))?
        .sync_all()
        .await
        .map_err(|error| RuntimeError::internal(format!("sync snapshot file: {error}")))
}

async fn sync_directory(path: &Path) -> Result<(), RuntimeError> {
    sync_file(path).await
}

async fn set_mode(path: &Path, mode: u32) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(path)
            .await
            .map_err(|error| RuntimeError::internal(format!("stat permissions: {error}")))?
            .permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions)
            .await
            .map_err(|error| RuntimeError::internal(format!("set permissions: {error}")))?;
    }
    Ok(())
}

fn integrity_error(message: impl Into<String>) -> RuntimeError {
    RuntimeError::new(RuntimeErrorKind::Unavailable, message)
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
