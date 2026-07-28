use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio::{fs, process::Command};

#[derive(Debug, Error)]
pub enum RootfsError {
    #[error("invalid rootfs path")]
    InvalidPath,
    #[error("rootfs operation failed: {0}")]
    Io(String),
}

pub async fn clone_rootfs(source: &Path, destination: &Path) -> Result<(), RootfsError> {
    if !source.is_absolute() || !destination.is_absolute() || source == destination {
        return Err(RootfsError::InvalidPath);
    }
    let parent = destination.parent().ok_or(RootfsError::InvalidPath)?;
    fs::create_dir_all(parent)
        .await
        .map_err(|error| RootfsError::Io(error.to_string()))?;

    #[cfg(target_os = "linux")]
    {
        let status = Command::new("cp")
            .arg("--reflink=auto")
            .arg("--sparse=always")
            .arg("--")
            .arg(source)
            .arg(destination)
            .status()
            .await
            .map_err(|error| RootfsError::Io(error.to_string()))?;
        if status.success() {
            return Ok(());
        }
    }

    fs::copy(source, destination)
        .await
        .map_err(|error| RootfsError::Io(error.to_string()))?;
    Ok(())
}

pub async fn verify_regular_file(path: &Path) -> Result<PathBuf, RootfsError> {
    let canonical = fs::canonicalize(path)
        .await
        .map_err(|error| RootfsError::Io(error.to_string()))?;
    let metadata = fs::metadata(&canonical)
        .await
        .map_err(|error| RootfsError::Io(error.to_string()))?;
    if !metadata.is_file() {
        return Err(RootfsError::InvalidPath);
    }
    Ok(canonical)
}
