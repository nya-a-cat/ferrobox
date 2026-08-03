use std::path::{Path, PathBuf};

use ferrobox_core::{RuntimeError, RuntimeErrorKind};
use ferrobox_template::{TemplateArtifact, TemplateCatalog, TemplateCatalogError};
use sha2::{Digest as _, Sha256};
use tokio::{fs, io::AsyncReadExt as _};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeTemplateAssets {
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct TemplateRuntimeResolver {
    catalog: TemplateCatalog,
    compatible_kernel_digest: String,
    architecture: String,
}

impl TemplateRuntimeResolver {
    pub(crate) fn new(catalog_root: PathBuf, compatible_kernel_digest: String) -> Self {
        Self {
            catalog: TemplateCatalog::new(catalog_root),
            compatible_kernel_digest,
            architecture: std::env::consts::ARCH.to_owned(),
        }
    }

    pub(crate) async fn resolve(
        &self,
        template_id: &str,
    ) -> Result<RuntimeTemplateAssets, RuntimeError> {
        let requested_id = template_id.to_owned();
        let record = self
            .catalog
            .record(&requested_id)
            .map_err(|error| map_catalog_error(&requested_id, error))?;

        if record.template_id != requested_id {
            return Err(RuntimeError::invalid(
                "runtime template selection requires a content-derived template ID",
            ));
        }
        let platform = &record.descriptor.platform;
        if platform.operating_system != "linux"
            || platform.runtime != "firecracker"
            || platform.architecture != self.architecture
        {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unsupported,
                "template platform is incompatible with this runtime",
            ));
        }
        if record.descriptor.artifacts.kernel.digest != self.compatible_kernel_digest {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unsupported,
                "template kernel is incompatible with this runtime snapshot contract",
            ));
        }
        if !artifact_matches(&record.locations.kernel, &record.descriptor.artifacts.kernel).await?
            || !artifact_matches(&record.locations.rootfs, &record.descriptor.artifacts.rootfs)
                .await?
        {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unavailable,
                "template artifact verification failed",
            ));
        }
        Ok(RuntimeTemplateAssets {
            kernel: record.locations.kernel,
            rootfs: record.locations.rootfs,
        })
    }
}

async fn artifact_matches(
    path: &Path,
    expected: &TemplateArtifact,
) -> Result<bool, RuntimeError> {
    let metadata = fs::metadata(path)
        .await
        .map_err(|_| template_artifact_unavailable())?;
    if !metadata.is_file() || metadata.len() != expected.size_bytes {
        return Ok(false);
    }
    let mut file = fs::File::open(path)
        .await
        .map_err(|_| template_artifact_unavailable())?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let length = file
            .read(&mut buffer)
            .await
            .map_err(|_| template_artifact_unavailable())?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("sha256:{:x}", digest.finalize()) == expected.digest)
}

fn template_artifact_unavailable() -> RuntimeError {
    RuntimeError::new(
        RuntimeErrorKind::Unavailable,
        "template artifact verification failed",
    )
}

fn map_catalog_error(template_id: &str, error: TemplateCatalogError) -> RuntimeError {
    match error {
        TemplateCatalogError::NotFound(_) => {
            RuntimeError::not_found(format!("template {template_id} was not found"))
        }
        TemplateCatalogError::InvalidInput { .. } => {
            RuntimeError::invalid("template identifier is invalid")
        }
        TemplateCatalogError::AliasConflict { .. }
        | TemplateCatalogError::IdentityConflict { .. }
        | TemplateCatalogError::CorruptRecord { .. }
        | TemplateCatalogError::Io { .. }
        | TemplateCatalogError::Serialization(_) => RuntimeError::new(
            RuntimeErrorKind::Unavailable,
            "template catalog inspection failed",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use ferrobox_core::RuntimeErrorKind;
    use ferrobox_template::{BuildTemplateRequest, TemplateCatalog};
    use tempfile::tempdir;

    use super::TemplateRuntimeResolver;

    fn request(root: &Path, alias: &str) -> BuildTemplateRequest {
        let kernel_path = root.join("vmlinux");
        let rootfs_path = root.join("rootfs.ext4");
        fs::write(&kernel_path, b"compatible-kernel").unwrap();
        fs::write(&rootfs_path, b"selected-rootfs").unwrap();
        BuildTemplateRequest {
            name: "python".to_owned(),
            version: "3.12.0".to_owned(),
            alias: alias.to_owned(),
            source_kind: "oci".to_owned(),
            source_reference: "docker.io/library/python@sha256:fixture".to_owned(),
            source_digest:
                "sha256:1111111111111111111111111111111111111111111111111111111111111111"
                    .to_owned(),
            target_architecture: std::env::consts::ARCH.to_owned(),
            kernel_path,
            rootfs_path,
        }
    }

    #[tokio::test]
    async fn resolves_verified_content_id_and_rejects_alias_selection() {
        let temporary = tempdir().unwrap();
        let assets = temporary.path().join("assets");
        let store = temporary.path().join("catalog");
        fs::create_dir(&assets).unwrap();
        let catalog = TemplateCatalog::new(&store);
        let built = catalog.build(request(&assets, "python-3-12")).unwrap();
        let resolver = TemplateRuntimeResolver::new(
            store,
            built.record.descriptor.artifacts.kernel.digest.clone(),
        );

        let resolved = resolver.resolve(&built.record.template_id).await.unwrap();
        assert_eq!(resolved.kernel, built.record.locations.kernel);
        assert_eq!(resolved.rootfs, built.record.locations.rootfs);

        let error = resolver.resolve("python-3-12").await.unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn rejects_missing_and_tampered_templates() {
        let temporary = tempdir().unwrap();
        let assets = temporary.path().join("assets");
        let store = temporary.path().join("catalog");
        fs::create_dir(&assets).unwrap();
        let catalog = TemplateCatalog::new(&store);
        let built = catalog.build(request(&assets, "python-3-12")).unwrap();
        let resolver = TemplateRuntimeResolver::new(
            store,
            built.record.descriptor.artifacts.kernel.digest.clone(),
        );

        let missing = resolver
            .resolve("tpl-000000000000000000000000000000000000000000000000000000000000")
            .await
            .unwrap_err();
        assert_eq!(missing.kind(), RuntimeErrorKind::NotFound);

        fs::write(&built.record.locations.rootfs, b"tampered-rootfs").unwrap();
        let tampered = resolver
            .resolve(&built.record.template_id)
            .await
            .unwrap_err();
        assert_eq!(tampered.kind(), RuntimeErrorKind::Unavailable);
    }

    #[tokio::test]
    async fn rejects_incompatible_kernel() {
        let temporary = tempdir().unwrap();
        let assets = temporary.path().join("assets");
        let store = temporary.path().join("catalog");
        fs::create_dir(&assets).unwrap();
        let catalog = TemplateCatalog::new(&store);
        let built = catalog.build(request(&assets, "python-3-12")).unwrap();
        let resolver = TemplateRuntimeResolver::new(
            store,
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                .to_owned(),
        );

        let error = resolver
            .resolve(&built.record.template_id)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::Unsupported);
    }
}
