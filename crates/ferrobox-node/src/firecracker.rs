use std::{path::PathBuf, time::Duration};

use bytes::Bytes;
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt as _, Full};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct FirecrackerClient {
    socket: PathBuf,
    timeout: Duration,
}

#[derive(Debug, Error)]
pub enum FirecrackerError {
    #[error("Firecracker API transport failed: {0}")]
    Transport(String),
    #[error("Firecracker API returned {status}: {body}")]
    Api { status: StatusCode, body: String },
    #[error("Firecracker API response was invalid: {0}")]
    Decode(String),
    #[error("Firecracker Unix sockets require a Unix host")]
    UnsupportedHost,
}

impl FirecrackerClient {
    #[must_use]
    pub const fn new(socket: PathBuf, timeout: Duration) -> Self {
        Self { socket, timeout }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, FirecrackerError> {
        let body = self.request(Method::GET, path, Bytes::new()).await?;
        serde_json::from_slice(&body).map_err(|error| FirecrackerError::Decode(error.to_string()))
    }

    pub async fn put<T: Serialize>(&self, path: &str, value: &T) -> Result<(), FirecrackerError> {
        self.send_json(Method::PUT, path, value).await
    }

    pub async fn patch<T: Serialize>(&self, path: &str, value: &T) -> Result<(), FirecrackerError> {
        self.send_json(Method::PATCH, path, value).await
    }

    async fn send_json<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        value: &T,
    ) -> Result<(), FirecrackerError> {
        let body = serde_json::to_vec(value)
            .map_err(|error| FirecrackerError::Decode(error.to_string()))?;
        self.request(method, path, Bytes::from(body)).await?;
        Ok(())
    }

    #[cfg(unix)]
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Bytes,
    ) -> Result<Bytes, FirecrackerError> {
        use hyper_util::client::legacy::Client;
        use hyperlocal::{UnixClientExt as _, UnixConnector, Uri};

        let client: Client<UnixConnector, Full<Bytes>> = Client::unix();
        let uri: hyper::Uri = Uri::new(&self.socket, path).into();
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Full::new(body))
            .map_err(|error| FirecrackerError::Transport(error.to_string()))?;
        let response = tokio::time::timeout(self.timeout, client.request(request))
            .await
            .map_err(|_| FirecrackerError::Transport("request timed out".to_owned()))?
            .map_err(|error| FirecrackerError::Transport(error.to_string()))?;
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|error| FirecrackerError::Transport(error.to_string()))?
            .to_bytes();
        if status.is_success() {
            Ok(bytes)
        } else {
            let fault = serde_json::from_slice::<Fault>(&bytes)
                .map(|value| value.fault_message)
                .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
            Err(FirecrackerError::Api {
                status,
                body: fault,
            })
        }
    }

    #[cfg(not(unix))]
    async fn request(
        &self,
        _method: Method,
        _path: &str,
        _body: Bytes,
    ) -> Result<Bytes, FirecrackerError> {
        Err(FirecrackerError::UnsupportedHost)
    }
}

#[derive(Debug, Deserialize)]
struct Fault {
    fault_message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MachineConfig {
    pub vcpu_count: u8,
    pub mem_size_mib: u32,
    pub smt: bool,
    pub track_dirty_pages: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BootSource {
    pub kernel_image_path: String,
    pub boot_args: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Drive {
    pub drive_id: String,
    pub path_on_host: String,
    pub is_root_device: bool,
    pub is_read_only: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct NetworkInterface {
    pub iface_id: String,
    pub host_dev_name: String,
    pub guest_mac: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Vsock {
    pub guest_cid: u32,
    pub uds_path: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub enum InstanceActionType {
    InstanceStart,
    SendCtrlAltDel,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstanceAction {
    pub action_type: InstanceActionType,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum VmStateValue {
    Paused,
    Resumed,
}

#[derive(Clone, Debug, Serialize)]
pub struct VmState {
    pub state: VmStateValue,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum SnapshotType {
    Full,
}

#[derive(Clone, Debug, Serialize)]
pub struct SnapshotCreate {
    pub snapshot_type: SnapshotType,
    pub snapshot_path: String,
    pub mem_file_path: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub enum MemoryBackendType {
    File,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryBackend {
    pub backend_path: String,
    pub backend_type: MemoryBackendType,
}

#[derive(Clone, Debug, Serialize)]
pub struct SnapshotLoad {
    pub snapshot_path: String,
    pub mem_backend: MemoryBackend,
    pub track_dirty_pages: bool,
    pub resume_vm: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VersionResponse {
    pub firecracker_version: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct InstanceInfo {
    pub id: String,
    pub state: String,
    pub vmm_version: String,
}
