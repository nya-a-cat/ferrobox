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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedRuntimeTemplate {
    pub template_id: String,
    pub assets: RuntimeTemplateAssets,
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
        template_reference: &str,
    ) -> Result<ResolvedRuntimeTemplate, RuntimeError> {
        let requested_reference = template_reference.to_owned();
        let record = self
            .catalog
            .record(&requested_reference)
            .map_err(|error| map_catalog_error(&requested_reference, error))?;

        if requested_reference.starts_with("tpl-") && record.template_id != requested_reference {
            return Err(RuntimeError::invalid(
                "tpl-prefixed runtime template selection requires a content-derived template ID",
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
        if !artifact_matches(
            &record.locations.kernel,
            &record.descriptor.artifacts.kernel,
        )
        .await?
            || !artifact_matches(
                &record.locations.rootfs,
                &record.descriptor.artifacts.rootfs,
            )
            .await?
        {
            return Err(RuntimeError::new(
                RuntimeErrorKind::Unavailable,
                "template artifact verification failed",
            ));
        }
        Ok(ResolvedRuntimeTemplate {
            template_id: record.template_id,
            assets: RuntimeTemplateAssets {
                kernel: record.locations.kernel,
                rootfs: record.locations.rootfs,
            },
        })
    }
}

async fn artifact_matches(path: &Path, expected: &TemplateArtifact) -> Result<bool, RuntimeError> {
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
                "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
            target_architecture: std::env::consts::ARCH.to_owned(),
            kernel_path,
            rootfs_path,
        }
    }

    #[tokio::test]
    async fn resolves_alias_and_content_id_to_the_same_immutable_template() {
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

        let by_id = resolver.resolve(&built.record.template_id).await.unwrap();
        let by_alias = resolver.resolve("python-3-12").await.unwrap();
        assert_eq!(by_id, by_alias);
        assert_eq!(by_alias.template_id, built.record.template_id);
        assert_eq!(by_alias.assets.kernel, built.record.locations.kernel);
        assert_eq!(by_alias.assets.rootfs, built.record.locations.rootfs);

        let missing_alias = resolver.resolve("missing-alias").await.unwrap_err();
        assert_eq!(missing_alias.kind(), RuntimeErrorKind::NotFound);
    }

    #[tokio::test]
    async fn reserves_tpl_prefix_for_content_ids() {
        let temporary = tempdir().unwrap();
        let assets = temporary.path().join("assets");
        let store = temporary.path().join("catalog");
        fs::create_dir(&assets).unwrap();
        let catalog = TemplateCatalog::new(&store);
        let built = catalog.build(request(&assets, "tpl-shadow")).unwrap();
        let resolver = TemplateRuntimeResolver::new(
            store,
            built.record.descriptor.artifacts.kernel.digest.clone(),
        );

        let error = resolver.resolve("tpl-shadow").await.unwrap_err();
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
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_owned(),
        );

        let error = resolver
            .resolve(&built.record.template_id)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), RuntimeErrorKind::Unsupported);
    }
}
