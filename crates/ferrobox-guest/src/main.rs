use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use ferrobox_guest::service::GuestService;
use ferrobox_protocol::guest::v1::guest_service_server::GuestServiceServer;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_vsock::{VsockAddr, VsockListener, VsockStream};
use tonic::transport::server::Connected;

struct ConnectedVsock(VsockStream);

impl Connected for ConnectedVsock {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl AsyncRead for ConnectedVsock {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for ConnectedVsock {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.0).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.0).poll_shutdown(context)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let port = std::env::var("FERROBOX_GUEST_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5000);
    let listener = VsockListener::bind(VsockAddr::new(libc::VMADDR_CID_ANY, port))?;
    let incoming = futures::stream::unfold(listener, |listener| async move {
        let item = listener
            .accept()
            .await
            .map(|(stream, _)| ConnectedVsock(stream));
        Some((item, listener))
    });
    tonic::transport::Server::builder()
        .add_service(GuestServiceServer::new(GuestService::new(
            "/home/sandbox".into(),
        )?))
        .serve_with_incoming(incoming)
        .await?;
    Ok(())
}
