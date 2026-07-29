use std::{
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use clap::Parser;
use ferrobox_node::vsock::GuestConnector;
use ferrobox_protocol::guest::v1::{
    Auth, HealthRequest, InitRequest, StartProcessRequest,
    guest_service_client::GuestServiceClient, process_event,
};
use serde::Serialize;
use tonic::{Request, transport::Channel};

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    vsock: PathBuf,
    #[arg(long)]
    launched_unix_nanos: u128,
    #[arg(long)]
    health_only: bool,
}

#[derive(Debug, Serialize)]
struct ProbeResult {
    schema_version: u32,
    ready_us: u128,
    exec_true_us: Vec<u128>,
    exec_true_p50_us: Option<u128>,
    exec_true_p95_us: Option<u128>,
    exec_true_cloned_client_us: Vec<u128>,
    exec_true_cloned_client_p50_us: Option<u128>,
    exec_true_cloned_client_p95_us: Option<u128>,
    exec_python_us: Vec<u128>,
    exec_python_p50_us: Option<u128>,
    exec_python_p95_us: Option<u128>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arguments = Args::parse();
    let connector = GuestConnector::new(arguments.vsock, 5000, Duration::from_secs(1));
    let mut client = wait_for_guest(&connector).await?;
    let ready_unix_nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let ready_us = ready_unix_nanos.saturating_sub(arguments.launched_unix_nanos) / 1000;

    if arguments.health_only {
        print_result(ready_us, Vec::new(), Vec::new(), Vec::new())?;
        return Ok(());
    }

    let token = "ferrobox-microvm-benchmark-token-00000000".to_owned();
    client
        .init(Request::new(InitRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            sandbox_id: uuid::Uuid::new_v4().to_string(),
            token: token.clone(),
            command_uid: 1000,
            command_gid: 1000,
            max_file_bytes: 64 * 1024 * 1024,
            max_processes: 512,
            guest_ipv4: String::new(),
            guest_prefix_length: 0,
            gateway_ipv4: String::new(),
            dns_ipv4: String::new(),
        }))
        .await?;

    let mut exec_true_us = Vec::with_capacity(100);
    for _ in 0..100 {
        exec_true_us.push(execute(&mut client, &token, vec!["/bin/true".to_owned()]).await?);
    }

    let mut exec_true_cloned_client_us = Vec::with_capacity(100);
    for _ in 0..100 {
        let mut cloned_client = client.clone();
        exec_true_cloned_client_us
            .push(execute(&mut cloned_client, &token, vec!["/bin/true".to_owned()]).await?);
    }

    execute(
        &mut client,
        &token,
        vec![
            "python3".to_owned(),
            "-c".to_owned(),
            "print(42)".to_owned(),
        ],
    )
    .await?;
    let mut exec_python_us = Vec::with_capacity(30);
    for _ in 0..30 {
        exec_python_us.push(
            execute(
                &mut client,
                &token,
                vec![
                    "python3".to_owned(),
                    "-c".to_owned(),
                    "print(42)".to_owned(),
                ],
            )
            .await?,
        );
    }
    print_result(
        ready_us,
        exec_true_us,
        exec_true_cloned_client_us,
        exec_python_us,
    )
}

async fn wait_for_guest(connector: &GuestConnector) -> anyhow::Result<GuestServiceClient<Channel>> {
    let started = Instant::now();
    loop {
        if let Ok(mut client) = connector.client().await
            && let Ok(response) = client.health(Request::new(HealthRequest {})).await
            && response.into_inner().ready
        {
            return Ok(client);
        }
        if started.elapsed() >= Duration::from_secs(30) {
            anyhow::bail!("guest did not become ready within 30 seconds");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn execute(
    client: &mut GuestServiceClient<Channel>,
    token: &str,
    argv: Vec<String>,
) -> anyhow::Result<u128> {
    let started = Instant::now();
    let response = client
        .start_process(Request::new(StartProcessRequest {
            auth: Some(Auth {
                token: token.to_owned(),
            }),
            request_id: uuid::Uuid::new_v4().to_string(),
            argv,
            cwd: "/home/sandbox".to_owned(),
            environment: Default::default(),
            timeout_millis: 30_000,
            max_output_bytes: 1024 * 1024,
        }))
        .await?;
    let mut stream = response.into_inner();
    let mut successful_exit = false;
    while let Some(event) = stream.message().await? {
        match event.event {
            Some(process_event::Event::Exit(exit)) if exit.exit_code == Some(0) => {
                successful_exit = true;
            }
            Some(process_event::Event::Exit(exit)) => {
                anyhow::bail!("guest command failed: {exit:?}");
            }
            Some(process_event::Event::Error(error)) => {
                anyhow::bail!("guest command error {}: {}", error.code, error.message);
            }
            _ => {}
        }
    }
    if !successful_exit {
        anyhow::bail!("guest command omitted a successful exit event");
    }
    Ok(started.elapsed().as_micros())
}

fn print_result(
    ready_us: u128,
    mut exec_true_us: Vec<u128>,
    mut exec_true_cloned_client_us: Vec<u128>,
    mut exec_python_us: Vec<u128>,
) -> anyhow::Result<()> {
    exec_true_us.sort_unstable();
    exec_true_cloned_client_us.sort_unstable();
    exec_python_us.sort_unstable();
    let result = ProbeResult {
        schema_version: 2,
        ready_us,
        exec_true_p50_us: percentile(&exec_true_us, 50),
        exec_true_p95_us: percentile(&exec_true_us, 95),
        exec_true_us,
        exec_true_cloned_client_p50_us: percentile(&exec_true_cloned_client_us, 50),
        exec_true_cloned_client_p95_us: percentile(&exec_true_cloned_client_us, 95),
        exec_true_cloned_client_us,
        exec_python_p50_us: percentile(&exec_python_us, 50),
        exec_python_p95_us: percentile(&exec_python_us, 95),
        exec_python_us,
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn percentile(samples: &[u128], value: usize) -> Option<u128> {
    if samples.is_empty() {
        return None;
    }
    let rank = (samples.len() * value).div_ceil(100);
    Some(samples[rank.saturating_sub(1).min(samples.len() - 1)])
}
