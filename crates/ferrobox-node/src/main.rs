use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use clap::{Args, Parser, Subcommand};
use ferrobox_core::{
    ExecRequest, ExecResult, ExecTermination, NetworkMode, SandboxPath, SandboxRuntime, SandboxSpec,
};
use ferrobox_node::{FirecrackerRuntime, FirecrackerRuntimeConfig};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "ferrobox-node", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Check(RuntimeArgs),
    Benchmark {
        #[arg(long, default_value_t = 5)]
        create_iterations: u32,
        #[arg(long, default_value_t = 20)]
        exec_iterations: u32,
        #[arg(long, default_value_t = 30)]
        python_iterations: u32,
        #[arg(long, default_value_t = 20)]
        file_iterations: u32,
        #[command(flatten)]
        runtime: RuntimeArgs,
    },
    RunTemplate {
        template: String,
        #[arg(long)]
        internet: bool,
        #[command(flatten)]
        runtime: RuntimeArgs,
    },
}

#[derive(Debug, Serialize)]
struct BenchmarkResult {
    schema_version: u32,
    pool_prepare_us: Vec<u128>,
    pool_prepare_p50_us: u128,
    pool_prepare_p95_us: u128,
    pool_firecracker_rss_kib: u64,
    pool_size: usize,
    create_to_ready_us: Vec<u128>,
    create_to_ready_p50_us: u128,
    create_to_ready_p95_us: u128,
    exec_true_us: Vec<u128>,
    exec_true_p50_us: u128,
    exec_true_p95_us: u128,
    exec_true_total_us: u128,
    exec_true_throughput_milli_ops_per_second: u128,
    exec_python_us: Vec<u128>,
    exec_python_p50_us: u128,
    exec_python_p95_us: u128,
    exec_python_total_us: u128,
    exec_python_throughput_milli_ops_per_second: u128,
    exec_file_roundtrip_us: Vec<u128>,
    exec_file_roundtrip_p50_us: u128,
    exec_file_roundtrip_p95_us: u128,
    exec_file_roundtrip_total_us: u128,
    exec_file_roundtrip_throughput_milli_ops_per_second: u128,
    delete_us: Vec<u128>,
    delete_p50_us: u128,
    delete_p95_us: u128,
    total_us: u128,
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
    #[arg(long, env = "FERROBOX_SNAPSHOT_ROOT")]
    snapshot_root: Option<PathBuf>,
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
            snapshot_root: self.snapshot_root.clone(),
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
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    match Cli::parse().command {
        Command::Benchmark {
            create_iterations,
            exec_iterations,
            python_iterations,
            file_iterations,
            runtime,
        } => {
            if create_iterations == 0 || create_iterations > 20 {
                anyhow::bail!("create-iterations must be between 1 and 20");
            }
            if exec_iterations == 0 || exec_iterations > 1000 {
                anyhow::bail!("exec-iterations must be between 1 and 1000");
            }
            if python_iterations == 0 || python_iterations > 1000 {
                anyhow::bail!("python-iterations must be between 1 and 1000");
            }
            if file_iterations == 0 || file_iterations > 1000 {
                anyhow::bail!("file-iterations must be between 1 and 1000");
            }
            let total_started = std::time::Instant::now();
            let runtime = FirecrackerRuntime::new(runtime.config()).await?;
            let mut pool_prepare_us = runtime
                .prewarm(benchmark_spec(), create_iterations as usize)
                .await?;
            pool_prepare_us.sort_unstable();
            let pool_size = runtime.ready_pool_len().await;
            let pool_firecracker_rss_kib = runtime.firecracker_rss_kib().await?;
            let mut create_to_ready_us = Vec::with_capacity(create_iterations as usize);
            let mut delete_us = Vec::with_capacity(create_iterations as usize);
            let mut handle = None;
            for iteration in 0..create_iterations {
                let create_started = std::time::Instant::now();
                let created = runtime.create(benchmark_spec()).await?;
                create_to_ready_us.push(create_started.elapsed().as_micros());
                if iteration + 1 == create_iterations {
                    handle = Some(created);
                } else {
                    let delete_started = std::time::Instant::now();
                    runtime.delete(&created.sandbox_id).await?;
                    delete_us.push(delete_started.elapsed().as_micros());
                }
            }
            create_to_ready_us.sort_unstable();
            let handle = handle.expect("positive create iteration count is validated");

            let mut exec_true_us = Vec::with_capacity(exec_iterations as usize);
            let exec_true_started = std::time::Instant::now();
            for _ in 0..exec_iterations {
                let started = std::time::Instant::now();
                ensure_exit_success(
                    &runtime
                        .execute(
                            &handle.sandbox_id,
                            exec_request(vec!["/bin/true".to_owned()]),
                        )
                        .await?,
                )?;
                exec_true_us.push(started.elapsed().as_micros());
            }
            let exec_true_total_us = exec_true_started.elapsed().as_micros();
            exec_true_us.sort_unstable();

            ensure_exit_success(
                &runtime
                    .execute(
                        &handle.sandbox_id,
                        exec_request(vec![
                            "python3".to_owned(),
                            "-c".to_owned(),
                            "print(42)".to_owned(),
                        ]),
                    )
                    .await?,
            )?;
            let mut exec_python_us = Vec::with_capacity(python_iterations as usize);
            let exec_python_started = std::time::Instant::now();
            for _ in 0..python_iterations {
                let started = std::time::Instant::now();
                ensure_exit_success(
                    &runtime
                        .execute(
                            &handle.sandbox_id,
                            exec_request(vec![
                                "python3".to_owned(),
                                "-c".to_owned(),
                                "print(42)".to_owned(),
                            ]),
                        )
                        .await?,
                )?;
                exec_python_us.push(started.elapsed().as_micros());
            }
            let exec_python_total_us = exec_python_started.elapsed().as_micros();
            exec_python_us.sort_unstable();

            let file_command = vec![
                "python3".to_owned(),
                "-c".to_owned(),
                "from pathlib import Path; p=Path('/tmp/ferrobox-bench.bin'); data=b'x'*1048576; p.write_bytes(data); assert p.read_bytes()==data; p.unlink()".to_owned(),
            ];
            ensure_exit_success(
                &runtime
                    .execute(&handle.sandbox_id, exec_request(file_command.clone()))
                    .await?,
            )?;
            let mut exec_file_roundtrip_us = Vec::with_capacity(file_iterations as usize);
            let exec_file_roundtrip_started = std::time::Instant::now();
            for _ in 0..file_iterations {
                let started = std::time::Instant::now();
                ensure_exit_success(
                    &runtime
                        .execute(&handle.sandbox_id, exec_request(file_command.clone()))
                        .await?,
                )?;
                exec_file_roundtrip_us.push(started.elapsed().as_micros());
            }
            let exec_file_roundtrip_total_us = exec_file_roundtrip_started.elapsed().as_micros();
            exec_file_roundtrip_us.sort_unstable();

            let delete_started = std::time::Instant::now();
            runtime.delete(&handle.sandbox_id).await?;
            delete_us.push(delete_started.elapsed().as_micros());
            delete_us.sort_unstable();
            let result = BenchmarkResult {
                schema_version: 8,
                pool_prepare_p50_us: percentile(&pool_prepare_us, 50),
                pool_prepare_p95_us: percentile(&pool_prepare_us, 95),
                pool_prepare_us,
                pool_firecracker_rss_kib,
                pool_size,
                create_to_ready_p50_us: percentile(&create_to_ready_us, 50),
                create_to_ready_p95_us: percentile(&create_to_ready_us, 95),
                create_to_ready_us,
                exec_true_p50_us: percentile(&exec_true_us, 50),
                exec_true_p95_us: percentile(&exec_true_us, 95),
                exec_true_total_us,
                exec_true_throughput_milli_ops_per_second: u128::from(exec_iterations)
                    .saturating_mul(1_000_000_000)
                    .checked_div(exec_true_total_us)
                    .unwrap_or_default(),
                exec_true_us,
                exec_python_p50_us: percentile(&exec_python_us, 50),
                exec_python_p95_us: percentile(&exec_python_us, 95),
                exec_python_total_us,
                exec_python_throughput_milli_ops_per_second: u128::from(python_iterations)
                    .saturating_mul(1_000_000_000)
                    .checked_div(exec_python_total_us)
                    .unwrap_or_default(),
                exec_python_us,
                exec_file_roundtrip_p50_us: percentile(&exec_file_roundtrip_us, 50),
                exec_file_roundtrip_p95_us: percentile(&exec_file_roundtrip_us, 95),
                exec_file_roundtrip_total_us,
                exec_file_roundtrip_throughput_milli_ops_per_second: u128::from(file_iterations)
                    .saturating_mul(1_000_000_000)
                    .checked_div(exec_file_roundtrip_total_us)
                    .unwrap_or_default(),
                exec_file_roundtrip_us,
                delete_p50_us: percentile(&delete_us, 50),
                delete_p95_us: percentile(&delete_us, 95),
                delete_us,
                total_us: total_started.elapsed().as_micros(),
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
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

fn benchmark_spec() -> SandboxSpec {
    SandboxSpec {
        template_id: "python".to_owned(),
        cpu_count: 1,
        memory_mb: 512,
        timeout_seconds: 300,
        network: NetworkMode::Disabled,
    }
}

fn exec_request(argv: Vec<String>) -> ExecRequest {
    ExecRequest {
        argv,
        cwd: SandboxPath::workspace(),
        environment: BTreeMap::new(),
        timeout_seconds: 30,
        max_output_bytes: 1024 * 1024,
    }
}

fn percentile(sorted: &[u128], percentile: usize) -> u128 {
    let rank = (sorted.len() * percentile).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted.len() - 1);
    sorted[index]
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

#[cfg(test)]
mod tests {
    use super::percentile;

    #[test]
    fn nearest_rank_p95_is_conservative_for_five_samples() {
        assert_eq!(percentile(&[1, 2, 3, 4, 5], 95), 5);
    }

    #[test]
    fn nearest_rank_p95_selects_nineteenth_of_twenty() {
        assert_eq!(percentile(&(1..=20).collect::<Vec<_>>(), 95), 19);
    }
}
