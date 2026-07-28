use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use clap::{Args, Parser, Subcommand};
use ferrobox_core::{
    ExecRequest, ExecResult, ExecTermination, NetworkMode, SandboxPath, SandboxRuntime, SandboxSpec,
};
use ferrobox_node::{FirecrackerRuntime, FirecrackerRuntimeConfig};

#[derive(Debug, Parser)]
#[command(name = "ferrobox-node", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Check(RuntimeArgs),
    RunTemplate {
        template: String,
        #[arg(long)]
        internet: bool,
        #[command(flatten)]
        runtime: RuntimeArgs,
    },
}

#[derive(Clone, Debug, Args)]
struct RuntimeArgs {
    #[arg(long, env = "FERROBOX_FIRECRACKER")]
    firecracker: PathBuf,
    #[arg(long, env = "FERROBOX_JAILER")]
    jailer: PathBuf,
    #[arg(long, env = "FERROBOX_KERNEL")]
    kernel: PathBuf,
    #[arg(long, env = "FERROBOX_ROOTFS")]
    rootfs: PathBuf,
    #[arg(
        long,
        env = "FERROBOX_CHROOT_BASE",
        default_value = "/srv/ferrobox/jailer"
    )]
    chroot_base: PathBuf,
    #[arg(
        long,
        env = "FERROBOX_RUNTIME_ROOT",
        default_value = "/var/lib/ferrobox/runtime"
    )]
    runtime_root: PathBuf,
    #[arg(long, env = "FERROBOX_JAIL_UID", default_value_t = 1001)]
    jail_uid: u32,
    #[arg(long, env = "FERROBOX_JAIL_GID", default_value_t = 1001)]
    jail_gid: u32,
}

impl RuntimeArgs {
    fn config(&self) -> FirecrackerRuntimeConfig {
        FirecrackerRuntimeConfig {
            firecracker_binary: self.firecracker.clone(),
            jailer_binary: self.jailer.clone(),
            kernel_image: self.kernel.clone(),
            rootfs_template: self.rootfs.clone(),
            chroot_base: self.chroot_base.clone(),
            runtime_root: self.runtime_root.clone(),
            jail_uid: self.jail_uid,
            jail_gid: self.jail_gid,
            guest_port: 5000,
            api_timeout: Duration::from_secs(5),
            boot_timeout: Duration::from_secs(30),
            node_id: "local-kvm".to_owned(),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    match Cli::parse().command {
        Command::Check(arguments) => {
            arguments.config().validate().await?;
            let kvm = tokio::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/kvm")
                .await?;
            drop(kvm);
            println!("Firecracker/KVM prerequisites are accessible");
        }
        Command::RunTemplate {
            template,
            internet,
            runtime,
        } => {
            let runtime = FirecrackerRuntime::new(runtime.config()).await?;
            let handle = runtime
                .create(SandboxSpec {
                    template_id: template,
                    cpu_count: 1,
                    memory_mb: 512,
                    timeout_seconds: 300,
                    network: if internet {
                        NetworkMode::Internet
                    } else {
                        NetworkMode::Disabled
                    },
                })
                .await?;
            let result = runtime
                .execute(
                    &handle.sandbox_id,
                    ExecRequest {
                        argv: vec![
                            "python3".to_owned(),
                            "-c".to_owned(),
                            "print(42)".to_owned(),
                        ],
                        cwd: SandboxPath::workspace(),
                        environment: BTreeMap::new(),
                        timeout_seconds: 30,
                        max_output_bytes: 1024 * 1024,
                    },
                )
                .await?;
            ensure_exit_success(&result)?;
            print!("{}", String::from_utf8_lossy(&result.stdout));
            if internet {
                let network_result = runtime
                    .execute(
                        &handle.sandbox_id,
                        ExecRequest {
                            argv: vec![
                                "python3".to_owned(),
                                "-c".to_owned(),
                                concat!(
                                    "import socket, urllib.request; ",
                                    "assert urllib.request.urlopen('https://example.com', timeout=10).status == 200; ",
                                    "s=socket.socket(); s.settimeout(2); ",
                                    "blocked=False; ",
                                    "\ntry: s.connect(('169.254.169.254', 80))", 
                                    "\nexcept OSError: blocked=True", 
                                    "\nassert blocked; print('internet=ok')"
                                )
                                .to_owned(),
                            ],
                            cwd: SandboxPath::workspace(),
                            environment: BTreeMap::new(),
                            timeout_seconds: 30,
                            max_output_bytes: 1024 * 1024,
                        },
                    )
                    .await?;
                ensure_exit_success(&network_result)?;
                print!("{}", String::from_utf8_lossy(&network_result.stdout));
            }
            runtime.delete(&handle.sandbox_id).await?;
        }
    }
    Ok(())
}

fn ensure_exit_success(result: &ExecResult) -> anyhow::Result<()> {
    match &result.termination {
        ExecTermination::Exited { exit_code: 0 } => Ok(()),
        _ => anyhow::bail!(
            "guest command failed: {:?}: {}",
            result.termination,
            String::from_utf8_lossy(&result.stderr)
        ),
    }
}