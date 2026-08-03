use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use clap::{Parser, ValueEnum};
use ferrobox_api::{AppState, router};
use ferrobox_node::{FirecrackerRuntime, FirecrackerRuntimeConfig, ProcessRuntime};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Backend {
    Firecracker,
    Process,
}

#[derive(Debug, Parser)]
#[command(name = "ferrobox-api", version)]
struct Cli {
    #[arg(long, value_enum, default_value = "firecracker")]
    backend: Backend,
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    #[arg(long, default_value = "/var/lib/ferrobox/audit/events.jsonl")]
    audit_log: PathBuf,
    #[arg(long, default_value = "/var/lib/ferrobox/process-runtime")]
    process_root: PathBuf,
    #[arg(long)]
    unsafe_process_runtime: bool,
    #[arg(long)]
    firecracker: Option<PathBuf>,
    #[arg(long)]
    jailer: Option<PathBuf>,
    #[arg(long)]
    kernel: Option<PathBuf>,
    #[arg(long)]
    rootfs: Option<PathBuf>,
    #[arg(long)]
    template_store: Option<PathBuf>,
    #[arg(long)]
    snapshot_root: Option<PathBuf>,
    #[arg(long, default_value_t = 0)]
    ready_pool_size: usize,
    #[arg(long, default_value = "/srv/ferrobox/jailer")]
    chroot_base: PathBuf,
    #[arg(long, default_value = "/var/lib/ferrobox/runtime")]
    runtime_root: PathBuf,
    #[arg(long, default_value_t = 1001)]
    jail_uid: u32,
    #[arg(long, default_value_t = 1001)]
    jail_gid: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let arguments = Cli::parse();
    let runtime: Arc<dyn ferrobox_core::SandboxRuntime> = match arguments.backend {
        Backend::Process => {
            if !arguments.unsafe_process_runtime {
                anyhow::bail!("process backend requires --unsafe-process-runtime");
            }
            if !arguments.listen.ip().is_loopback() {
                anyhow::bail!("process backend may bind only to loopback");
            }
            Arc::new(ProcessRuntime::new(arguments.process_root).await?)
        }
        Backend::Firecracker => {
            let runtime = Arc::new(
                FirecrackerRuntime::new(FirecrackerRuntimeConfig {
                    firecracker_binary: required(arguments.firecracker, "--firecracker")?,
                    jailer_binary: required(arguments.jailer, "--jailer")?,
                    kernel_image: required(arguments.kernel, "--kernel")?,
                    rootfs_template: required(arguments.rootfs, "--rootfs")?,
                    template_store: arguments.template_store,
                    snapshot_root: arguments.snapshot_root,
                    chroot_base: arguments.chroot_base,
                    runtime_root: arguments.runtime_root,
                    jail_uid: arguments.jail_uid,
                    jail_gid: arguments.jail_gid,
                    guest_port: 5000,
                    api_timeout: Duration::from_secs(5),
                    boot_timeout: Duration::from_secs(30),
                    node_id: "local-kvm".to_owned(),
                })
                .await?,
            );
            if arguments.ready_pool_size > 0 {
                let pool_spec = ferrobox_core::SandboxSpec {
                    template_id: "python".to_owned(),
                    cpu_count: 1,
                    memory_mb: 512,
                    timeout_seconds: 300,
                    network: ferrobox_core::NetworkMode::Disabled,
                };
                runtime
                    .prewarm(pool_spec.clone(), arguments.ready_pool_size)
                    .await?;
                let maintainer = Arc::clone(&runtime);
                let target_size = arguments.ready_pool_size;
                tokio::spawn(async move {
                    loop {
                        let missing = target_size.saturating_sub(maintainer.ready_pool_len().await);
                        if missing > 0
                            && let Err(error) = maintainer.prewarm(pool_spec.clone(), missing).await
                        {
                            tracing::error!(%error, "ready pool replenishment failed");
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        } else {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                        }
                    }
                });
            }
            runtime
        }
    };
    let state = AppState::new(runtime, arguments.audit_log).await?;
    let listener = tokio::net::TcpListener::bind(arguments.listen).await?;
    tracing::info!(address = %arguments.listen, "Ferrobox API listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

fn required(value: Option<PathBuf>, flag: &str) -> anyhow::Result<PathBuf> {
    value.ok_or_else(|| anyhow::anyhow!("{flag} is required for Firecracker backend"))
}
