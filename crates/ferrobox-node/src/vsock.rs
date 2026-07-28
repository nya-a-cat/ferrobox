use std::{path::PathBuf, time::Duration};

use ferrobox_protocol::guest::v1::guest_service_client::GuestServiceClient;
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::UnixStream,
};
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;

const MAX_HANDSHAKE_LINE: usize = 128;

#[derive(Clone, Debug)]
pub struct GuestConnector {
    uds_path: PathBuf,
    guest_port: u32,
    timeout: Duration,
}

#[derive(Debug, Error)]
pub enum VsockConnectError {
    #[error("vsock transport requires a Unix host")]
    UnsupportedHost,
    #[error("vsock connection failed: {0}")]
    Io(String),
    #[error("vsock proxy rejected connection: {0}")]
    Rejected(String),
    #[error("guest gRPC channel failed: {0}")]
    Tonic(String),
}

impl GuestConnector {
    #[must_use]
    pub const fn new(uds_path: PathBuf, guest_port: u32, timeout: Duration) -> Self {
        Self {
            uds_path,
            guest_port,
            timeout,
        }
    }

    #[cfg(unix)]
    pub async fn connect_stream(&self) -> Result<UnixStream, VsockConnectError> {
        let stream = tokio::time::timeout(self.timeout, UnixStream::connect(&self.uds_path))
            .await
            .map_err(|_| VsockConnectError::Io("connect timed out".to_owned()))?
            .map_err(|error| VsockConnectError::Io(error.to_string()))?;
        let mut reader = BufReader::new(stream);
        reader
            .get_mut()
            .write_all(format!("CONNECT {}\n", self.guest_port).as_bytes())
            .await
            .map_err(|error| VsockConnectError::Io(error.to_string()))?;
        reader
            .get_mut()
            .flush()
            .await
            .map_err(|error| VsockConnectError::Io(error.to_string()))?;
        let mut acknowledgement = String::new();
        let bytes = tokio::time::timeout(self.timeout, reader.read_line(&mut acknowledgement))
            .await
            .map_err(|_| VsockConnectError::Io("handshake timed out".to_owned()))?
            .map_err(|error| VsockConnectError::Io(error.to_string()))?;
        if bytes == 0
            || bytes > MAX_HANDSHAKE_LINE
            || !acknowledgement.starts_with("OK ")
            || !acknowledgement.ends_with('\n')
        {
            return Err(VsockConnectError::Rejected(
                acknowledgement.trim().to_owned(),
            ));
        }
        Ok(reader.into_inner())
    }

    #[cfg(not(unix))]
    pub async fn connect_stream(&self) -> Result<(), VsockConnectError> {
        Err(VsockConnectError::UnsupportedHost)
    }

    #[cfg(unix)]
    pub async fn client(&self) -> Result<GuestServiceClient<Channel>, VsockConnectError> {
        let connector = self.clone();
        let channel = Endpoint::from_static("http://[::]:5000")
            .connect_with_connector(service_fn(move |_| {
                let connector = connector.clone();
                async move {
                    connector
                        .connect_stream()
                        .await
                        .map_err(std::io::Error::other)
                }
            }))
            .await
            .map_err(|error| VsockConnectError::Tonic(error.to_string()))?;
        Ok(GuestServiceClient::new(channel))
    }

    #[cfg(not(unix))]
    pub async fn client(&self) -> Result<GuestServiceClient<Channel>, VsockConnectError> {
        Err(VsockConnectError::UnsupportedHost)
    }
}
