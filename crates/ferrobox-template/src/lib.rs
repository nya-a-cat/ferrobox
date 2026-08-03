//! Immutable, content-addressed template catalog.

use std::{
    ffi::OsStr,
    fmt::Write as _,
    fs::{self, File},
    io::{self, BufReader, Read, Write as _},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

const CATALOG_SCHEMA_VERSION: u32 = 1;
const TEMPLATE_CONTRACT: &str = "ferrobox-template-v1";
const TEMPLATE_ID_HEX_LENGTH: usize = 60;
const READY_STATUS: &str = "ready";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildTemplateRequest {
    pub name: String,
    pub version: String,
    pub alias: String,
    pub source_kind: String,
    pub source_reference: String,
    pub source_digest: String,
    pub target_architecture: String,
    pub kernel_path: PathBuf,
    pub rootfs_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TemplateRecord {
    pub catalog_schema_version: u32,
    pub template_id: String,
    pub alias: String,
    pub status: String,
    pub spec_digest: String,
    pub descriptor: TemplateDescriptor,
    pub locations: ArtifactLocations,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TemplateDescriptor {
    pub contract: String,
    pub name: String,
    pub version: String,
    pub source: TemplateSource,
    pub platform: TemplatePlatform,
    pub artifacts: TemplateArtifacts,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TemplateSource {
    pub kind: TemplateSourceKind,
    pub reference: String,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateSourceKind {
    Oci,
    File,
    Snapshot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TemplatePlatform {
    pub operating_system: String,
    pub architecture: String,
    pub runtime: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TemplateArtifacts {
    pub kernel: TemplateArtifact,
    pub rootfs: TemplateArtifact,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TemplateArtifact {
    pub media_type: String,
    pub digest: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtifactLocations {
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TemplateSummary {
    pub template_id: String,
    pub alias: String,
    pub name: String,
    pub version: String,
    pub status: String,
    pub source: TemplateSource,
    pub platform: TemplatePlatform,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TemplateInspection {
    pub record: TemplateRecord,
    pub verification: TemplateVerification,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TemplateVerification {
    pub valid: bool,
    pub descriptor_valid: bool,
    pub kernel: ArtifactVerification,
    pub rootfs: ArtifactVerification,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArtifactVerification {
    pub present: bool,
    pub valid: bool,
    pub expected_digest: String,
    pub actual_digest: Option<String>,
    pub expected_size_bytes: u64,
    pub actual_size_bytes: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeleteTemplateResult {
    pub template_id: String,
    pub alias: String,
    pub artifacts_preserved: bool,
    pub locations: ArtifactLocations,
}

#[derive(Debug, Error)]
pub enum TemplateCatalogError {
    #[error("invalid {field}: {reason}")]
    InvalidInput { field: &'static str, reason: String },
    #[error("template alias {alias} already points to {template_id}")]
    AliasConflict { alias: String, template_id: String },
    #[error("template identity {template_id} already uses alias {alias}")]
    IdentityConflict { template_id: String, alias: String },
    #[error("template {0} was not found")]
    NotFound(String),
    #[error("template catalog record {path} is invalid: {reason}")]
    CorruptRecord { path: PathBuf, reason: String },
    #[error("I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("template catalog serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
pub struct TemplateCatalog {
    root: PathBuf,
}

impl TemplateCatalog {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn build(
        &self,
        request: BuildTemplateRequest,
    ) -> Result<TemplateInspection, TemplateCatalogError> {
        let alias = validate_alias(&request.alias)?;
        let name = validate_text("name", &request.name, 128)?;
        let version = validate_version(&request.version)?;
        let source_reference = validate_text("source_reference", &request.source_reference, 1024)?;
        let source_digest = normalize_sha256("source_digest", &request.source_digest)?;
        let source_kind = parse_source_kind(&request.source_kind)?;
        let architecture = normalize_architecture(&request.target_architecture)?;
        let kernel_path = canonical_file("kernel", &request.kernel_path)?;
        let rootfs_path = canonical_file("rootfs", &request.rootfs_path)?;
        let (kernel_digest, kernel_size) = hash_file(&kernel_path)?;
        let (rootfs_digest, rootfs_size) = hash_file(&rootfs_path)?;

        let descriptor = TemplateDescriptor {
            contract: TEMPLATE_CONTRACT.to_owned(),
            name,
            version,
            source: TemplateSource {
                kind: source_kind,
                reference: source_reference,
                digest: source_digest,
            },
            platform: TemplatePlatform {
                operating_system: "linux".to_owned(),
                architecture,
                runtime: "firecracker".to_owned(),
            },
            artifacts: TemplateArtifacts {
                kernel: TemplateArtifact {
                    media_type: "application/vnd.ferrobox.kernel".to_owned(),
                    digest: kernel_digest,
                    size_bytes: kernel_size,
                },
                rootfs: TemplateArtifact {
                    media_type: "application/vnd.ferrobox.rootfs.ext4".to_owned(),
                    digest: rootfs_digest,
                    size_bytes: rootfs_size,
                },
            },
        };
        let spec_digest = descriptor_digest(&descriptor)?;
        let template_id = template_id_from_digest(&spec_digest);
        let record = TemplateRecord {
            catalog_schema_version: CATALOG_SCHEMA_VERSION,
            template_id: template_id.clone(),
            alias: alias.clone(),
            status: READY_STATUS.to_owned(),
            spec_digest,
            descriptor,
            locations: ArtifactLocations {
                kernel: kernel_path,
                rootfs: rootfs_path,
            },
        };

        for existing in self.records()? {
            if existing.alias == alias {
                if existing.template_id == template_id {
                    return self.inspect(&alias);
                }
                return Err(TemplateCatalogError::AliasConflict {
                    alias,
                    template_id: existing.template_id,
                });
            }
            if existing.template_id == template_id {
                return Err(TemplateCatalogError::IdentityConflict {
                    template_id,
                    alias: existing.alias,
                });
            }
        }

        self.persist(&record)?;
        self.inspect(&record.alias)
    }

    pub fn list(&self) -> Result<Vec<TemplateSummary>, TemplateCatalogError> {
        let mut summaries = self
            .records()?
            .into_iter()
            .map(|record| TemplateSummary {
                template_id: record.template_id,
                alias: record.alias,
                name: record.descriptor.name,
                version: record.descriptor.version,
                status: record.status,
                source: record.descriptor.source,
                platform: record.descriptor.platform,
            })
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| left.alias.cmp(&right.alias));
        Ok(summaries)
    }

    pub fn record(&self, reference: &str) -> Result<TemplateRecord, TemplateCatalogError> {
        self.resolve(reference).map(|(_, record)| record)
    }

    pub fn inspect(&self, reference: &str) -> Result<TemplateInspection, TemplateCatalogError> {
        let record = self.record(reference)?;
        let kernel = verify_artifact(
            &record.locations.kernel,
            &record.descriptor.artifacts.kernel,
        );
        let rootfs = verify_artifact(
            &record.locations.rootfs,
            &record.descriptor.artifacts.rootfs,
        );
        Ok(TemplateInspection {
            verification: TemplateVerification {
                valid: kernel.valid && rootfs.valid,
                descriptor_valid: true,
                kernel,
                rootfs,
            },
            record,
        })
    }

    pub fn delete(&self, reference: &str) -> Result<DeleteTemplateResult, TemplateCatalogError> {
        let (path, record) = self.resolve(reference)?;
        fs::remove_file(&path).map_err(|source| TemplateCatalogError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(DeleteTemplateResult {
            template_id: record.template_id,
            alias: record.alias,
            artifacts_preserved: true,
            locations: record.locations,
        })
    }

    fn records_dir(&self) -> PathBuf {
        self.root.join("records")
    }

    fn records(&self) -> Result<Vec<TemplateRecord>, TemplateCatalogError> {
        let directory = self.records_dir();
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(&directory).map_err(|source| TemplateCatalogError::Io {
            path: directory.clone(),
            source,
        })?;
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| TemplateCatalogError::Io {
                path: directory.clone(),
                source,
            })?;
            let path = entry.path();
            if path.extension() == Some(OsStr::new("json")) {
                records.push(load_record(&path)?);
            }
        }
        Ok(records)
    }

    fn resolve(&self, reference: &str) -> Result<(PathBuf, TemplateRecord), TemplateCatalogError> {
        let reference = validate_alias(reference)?;
        let direct = self.records_dir().join(format!("{reference}.json"));
        if direct.is_file() {
            return Ok((direct.clone(), load_record(&direct)?));
        }
        for record in self.records()? {
            if record.template_id == reference {
                let path = self.records_dir().join(format!("{}.json", record.alias));
                return Ok((path, record));
            }
        }
        Err(TemplateCatalogError::NotFound(reference))
    }

    fn persist(&self, record: &TemplateRecord) -> Result<(), TemplateCatalogError> {
        let directory = self.records_dir();
        fs::create_dir_all(&directory).map_err(|source| TemplateCatalogError::Io {
            path: directory.clone(),
            source,
        })?;
        let target = directory.join(format!("{}.json", record.alias));
        let mut payload = serde_json::to_vec_pretty(record)?;
        payload.push(b'\n');
        let mut temporary =
            NamedTempFile::new_in(&directory).map_err(|source| TemplateCatalogError::Io {
                path: directory.clone(),
                source,
            })?;
        temporary
            .write_all(&payload)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|source| TemplateCatalogError::Io {
                path: temporary.path().to_path_buf(),
                source,
            })?;
        temporary.persist_noclobber(&target).map_err(|error| {
            let source = error.error;
            if source.kind() == io::ErrorKind::AlreadyExists {
                TemplateCatalogError::AliasConflict {
                    alias: record.alias.clone(),
                    template_id: record.template_id.clone(),
                }
            } else {
                TemplateCatalogError::Io {
                    path: target.clone(),
                    source,
                }
            }
        })?;
        Ok(())
    }
}

fn parse_source_kind(value: &str) -> Result<TemplateSourceKind, TemplateCatalogError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "oci" => Ok(TemplateSourceKind::Oci),
        "file" => Ok(TemplateSourceKind::File),
        "snapshot" => Ok(TemplateSourceKind::Snapshot),
        _ => Err(TemplateCatalogError::InvalidInput {
            field: "source_kind",
            reason: "expected oci, file, or snapshot".to_owned(),
        }),
    }
}

fn normalize_architecture(value: &str) -> Result<String, TemplateCatalogError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "amd64" | "x86_64" => Ok("x86_64".to_owned()),
        "arm64" | "aarch64" => Ok("aarch64".to_owned()),
        _ => Err(TemplateCatalogError::InvalidInput {
            field: "target_arch",
            reason: "expected x86_64/amd64 or aarch64/arm64".to_owned(),
        }),
    }
}

fn validate_alias(value: &str) -> Result<String, TemplateCatalogError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(TemplateCatalogError::InvalidInput {
            field: "template reference",
            reason: "expected 1-64 ASCII letters, digits, hyphens, or underscores".to_owned(),
        });
    }
    Ok(value.to_owned())
}

fn validate_text(
    field: &'static str,
    value: &str,
    maximum_bytes: usize,
) -> Result<String, TemplateCatalogError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum_bytes || value.chars().any(char::is_control) {
        return Err(TemplateCatalogError::InvalidInput {
            field,
            reason: format!("expected 1-{maximum_bytes} bytes without control characters"),
        });
    }
    Ok(value.to_owned())
}

fn validate_version(value: &str) -> Result<String, TemplateCatalogError> {
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || byte == b'.'
                || byte == b'-'
                || byte == b'_'
                || byte == b'+'
        })
    {
        return Err(TemplateCatalogError::InvalidInput {
            field: "version",
            reason:
                "expected 1-64 ASCII letters, digits, dots, hyphens, underscores, or plus signs"
                    .to_owned(),
        });
    }
    Ok(value.to_owned())
}

fn normalize_sha256(field: &'static str, value: &str) -> Result<String, TemplateCatalogError> {
    let value = value.trim().to_ascii_lowercase();
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(TemplateCatalogError::InvalidInput {
            field,
            reason: "expected sha256:<64 hex characters>".to_owned(),
        });
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(TemplateCatalogError::InvalidInput {
            field,
            reason: "expected sha256:<64 hex characters>".to_owned(),
        });
    }
    Ok(value)
}

fn canonical_file(field: &'static str, path: &Path) -> Result<PathBuf, TemplateCatalogError> {
    let canonical = fs::canonicalize(path).map_err(|source| TemplateCatalogError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = fs::metadata(&canonical).map_err(|source| TemplateCatalogError::Io {
        path: canonical.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(TemplateCatalogError::InvalidInput {
            field,
            reason: format!("{} is not a regular file", canonical.display()),
        });
    }
    Ok(canonical)
}

fn descriptor_digest(descriptor: &TemplateDescriptor) -> Result<String, TemplateCatalogError> {
    let payload = serde_json::to_vec(descriptor)?;
    Ok(sha256_bytes(&payload))
}

fn template_id_from_digest(digest: &str) -> String {
    let hex = digest
        .strip_prefix("sha256:")
        .expect("internal template digests always use sha256");
    format!("tpl-{}", &hex[..TEMPLATE_ID_HEX_LENGTH])
}

fn hash_file(path: &Path) -> Result<(String, u64), TemplateCatalogError> {
    let file = File::open(path).map_err(|source| TemplateCatalogError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| TemplateCatalogError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    let digest = hasher.finalize();
    Ok((format_digest(&digest), size))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format_digest(&digest)
}

fn format_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(71);
    output.push_str("sha256:");
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn load_record(path: &Path) -> Result<TemplateRecord, TemplateCatalogError> {
    let payload = fs::read(path).map_err(|source| TemplateCatalogError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let record: TemplateRecord =
        serde_json::from_slice(&payload).map_err(|error| TemplateCatalogError::CorruptRecord {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
    validate_record(path, &record)?;
    Ok(record)
}

fn validate_record(path: &Path, record: &TemplateRecord) -> Result<(), TemplateCatalogError> {
    let fail = |reason: String| TemplateCatalogError::CorruptRecord {
        path: path.to_path_buf(),
        reason,
    };
    if record.catalog_schema_version != CATALOG_SCHEMA_VERSION {
        return Err(fail(format!(
            "unsupported schema version {}",
            record.catalog_schema_version
        )));
    }
    if record.status != READY_STATUS {
        return Err(fail(format!("unsupported status {}", record.status)));
    }
    if record.descriptor.contract != TEMPLATE_CONTRACT {
        return Err(fail(format!(
            "unsupported contract {}",
            record.descriptor.contract
        )));
    }
    validate_alias(&record.alias).map_err(|error| fail(error.to_string()))?;
    let expected_file_name = format!("{}.json", record.alias);
    if path.file_name() != Some(OsStr::new(&expected_file_name)) {
        return Err(fail("alias does not match the record file name".to_owned()));
    }
    let expected_digest = descriptor_digest(&record.descriptor)?;
    if record.spec_digest != expected_digest {
        return Err(fail("descriptor digest mismatch".to_owned()));
    }
    let expected_id = template_id_from_digest(&expected_digest);
    if record.template_id != expected_id {
        return Err(fail("template identity mismatch".to_owned()));
    }
    Ok(())
}

fn verify_artifact(path: &Path, expected: &TemplateArtifact) -> ArtifactVerification {
    match hash_file(path) {
        Ok((actual_digest, actual_size)) => ArtifactVerification {
            present: true,
            valid: actual_digest == expected.digest && actual_size == expected.size_bytes,
            expected_digest: expected.digest.clone(),
            actual_digest: Some(actual_digest),
            expected_size_bytes: expected.size_bytes,
            actual_size_bytes: Some(actual_size),
            error: None,
        },
        Err(error) => ArtifactVerification {
            present: path.is_file(),
            valid: false,
            expected_digest: expected.digest.clone(),
            actual_digest: None,
            expected_size_bytes: expected.size_bytes,
            actual_size_bytes: None,
            error: Some(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::{BuildTemplateRequest, TemplateCatalog, TemplateCatalogError};

    fn fixture_request(root: &Path, alias: &str) -> BuildTemplateRequest {
        let kernel_path = root.join("vmlinux");
        let rootfs_path = root.join("rootfs.ext4");
        fs::write(&kernel_path, b"kernel-v1").unwrap();
        fs::write(&rootfs_path, b"rootfs-v1").unwrap();
        BuildTemplateRequest {
            name: "python".to_owned(),
            version: "3.12.0".to_owned(),
            alias: alias.to_owned(),
            source_kind: "oci".to_owned(),
            source_reference: "docker.io/library/python:3.12-slim".to_owned(),
            source_digest:
                "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
            target_architecture: "amd64".to_owned(),
            kernel_path,
            rootfs_path,
        }
    }

    #[test]
    fn lifecycle_preserves_identity_and_source_artifacts() {
        let temporary = tempdir().unwrap();
        let assets = temporary.path().join("assets");
        fs::create_dir(&assets).unwrap();
        let catalog = TemplateCatalog::new(temporary.path().join("catalog"));
        let request = fixture_request(&assets, "python-3-12");

        let built = catalog.build(request.clone()).unwrap();
        assert_eq!(built.record.template_id.len(), 64);
        assert!(built.verification.valid);
        assert_eq!(built.record.descriptor.platform.architecture, "x86_64");
        assert_eq!(catalog.list().unwrap().len(), 1);
        assert_eq!(
            catalog
                .inspect(&built.record.template_id)
                .unwrap()
                .record
                .alias,
            "python-3-12"
        );

        let deleted = catalog.delete("python-3-12").unwrap();
        assert!(deleted.artifacts_preserved);
        assert!(request.kernel_path.is_file());
        assert!(request.rootfs_path.is_file());
        assert!(catalog.list().unwrap().is_empty());

        let rebuilt = catalog.build(request).unwrap();
        assert_eq!(rebuilt.record.template_id, built.record.template_id);
        assert_eq!(rebuilt.record.spec_digest, built.record.spec_digest);
    }

    #[test]
    fn identity_is_independent_of_artifact_locations() {
        let first = tempdir().unwrap();
        let second = tempdir().unwrap();
        let first_assets = first.path().join("assets");
        let second_assets = second.path().join("other-assets");
        fs::create_dir(&first_assets).unwrap();
        fs::create_dir(&second_assets).unwrap();
        let first_catalog = TemplateCatalog::new(first.path().join("catalog"));
        let second_catalog = TemplateCatalog::new(second.path().join("catalog"));

        let first_record = first_catalog
            .build(fixture_request(&first_assets, "python-3-12"))
            .unwrap();
        let second_record = second_catalog
            .build(fixture_request(&second_assets, "python-3-12"))
            .unwrap();

        assert_eq!(
            first_record.record.template_id,
            second_record.record.template_id
        );
        assert_eq!(
            first_record.record.spec_digest,
            second_record.record.spec_digest
        );
        assert_ne!(
            first_record.record.locations,
            second_record.record.locations
        );
    }

    #[test]
    fn alias_and_identity_are_immutable() {
        let temporary = tempdir().unwrap();
        let assets = temporary.path().join("assets");
        fs::create_dir(&assets).unwrap();
        let catalog = TemplateCatalog::new(temporary.path().join("catalog"));
        let request = fixture_request(&assets, "python-3-12");
        let built = catalog.build(request.clone()).unwrap();

        assert_eq!(
            catalog.build(request.clone()).unwrap().record.template_id,
            built.record.template_id
        );

        let mut changed = request.clone();
        changed.version = "3.12.1".to_owned();
        assert!(matches!(
            catalog.build(changed),
            Err(TemplateCatalogError::AliasConflict { .. })
        ));

        let mut renamed = request;
        renamed.alias = "stable".to_owned();
        assert!(matches!(
            catalog.build(renamed),
            Err(TemplateCatalogError::IdentityConflict { .. })
        ));
    }

    #[test]
    fn inspection_detects_artifact_tampering() {
        let temporary = tempdir().unwrap();
        let assets = temporary.path().join("assets");
        fs::create_dir(&assets).unwrap();
        let catalog = TemplateCatalog::new(temporary.path().join("catalog"));
        let request = fixture_request(&assets, "python-3-12");
        catalog.build(request.clone()).unwrap();

        fs::write(&request.rootfs_path, b"tampered-rootfs").unwrap();
        let inspection = catalog.inspect("python-3-12").unwrap();
        assert!(!inspection.verification.valid);
        assert!(inspection.verification.kernel.valid);
        assert!(!inspection.verification.rootfs.valid);
    }
}
