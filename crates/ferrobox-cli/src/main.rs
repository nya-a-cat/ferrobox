use std::path::PathBuf;

use anyhow::Context as _;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand};
use reqwest::{Client, Method, Response};
use serde_json::{Value, json};

#[derive(Debug, Parser)]
#[command(name = "ferrobox", version)]
struct Cli {
    #[arg(
        long,
        env = "FERROBOX_API_URL",
        default_value = "http://127.0.0.1:8080"
    )]
    api_url: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Create {
        #[arg(long, default_value = "python")]
        template: String,
        #[arg(long, default_value_t = 1)]
        cpu: u8,
        #[arg(long, default_value_t = 512)]
        memory_mb: u32,
        #[arg(long, default_value_t = 300)]
        ttl: u64,
        #[arg(long)]
        internet: bool,
    },
    Inspect {
        sandbox_id: String,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
    },
    Exec {
        sandbox_id: String,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
        #[arg(long, default_value = "/home/sandbox")]
        cwd: String,
        #[arg(long, default_value_t = 30)]
        timeout: u64,
        #[arg(required = true, trailing_var_arg = true)]
        argv: Vec<String>,
    },
    Write {
        sandbox_id: String,
        remote_path: String,
        local_path: PathBuf,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
        #[arg(long)]
        overwrite: bool,
    },
    Read {
        sandbox_id: String,
        remote_path: String,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    List {
        sandbox_id: String,
        #[arg(default_value = "/home/sandbox")]
        remote_path: String,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
    },
    Pause {
        sandbox_id: String,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
    },
    Resume {
        sandbox_id: String,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
    },
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
    },
    Delete {
        sandbox_id: String,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
    },
}

#[derive(Debug, Subcommand)]
enum SnapshotCommand {
    Create {
        sandbox_id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
    },
    List {
        sandbox_id: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
    },
    Inspect {
        snapshot_id: String,
        #[arg(long, env = "FERROBOX_SNAPSHOT_TOKEN")]
        token: String,
    },
    Verify {
        snapshot_id: String,
        #[arg(long, env = "FERROBOX_SNAPSHOT_TOKEN")]
        token: String,
    },
    Restore {
        snapshot_id: String,
        #[arg(long, default_value_t = 300)]
        ttl: u64,
        #[arg(long, env = "FERROBOX_SNAPSHOT_TOKEN")]
        token: String,
    },
    Clone {
        snapshot_id: String,
        #[arg(long)]
        count: u8,
        #[arg(long, default_value_t = 300)]
        ttl: u64,
        #[arg(long, env = "FERROBOX_SNAPSHOT_TOKEN")]
        token: String,
    },
    Rollback {
        sandbox_id: String,
        snapshot_id: String,
        #[arg(long, env = "FERROBOX_TOKEN")]
        token: String,
    },
    Delete {
        snapshot_id: String,
        #[arg(long, env = "FERROBOX_SNAPSHOT_TOKEN")]
        token: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arguments = Cli::parse();
    let client = Client::new();
    let base = arguments.api_url.trim_end_matches('/');
    match arguments.command {
        Command::Create {
            template,
            cpu,
            memory_mb,
            ttl,
            internet,
        } => {
            let response = client
                .post(format!("{base}/v1/sandboxes"))
                .json(&json!({
                    "template": template,
                    "cpu_count": cpu,
                    "memory_mb": memory_mb,
                    "timeout_seconds": ttl,
                    "network": { "internet_access": internet }
                }))
                .send()
                .await?;
            print_json(ensure_success(response).await?).await?;
        }
        Command::Inspect { sandbox_id, token } => {
            let response = authorized(
                &client,
                Method::GET,
                format!("{base}/v1/sandboxes/{sandbox_id}"),
                &token,
            )
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        Command::Exec {
            sandbox_id,
            token,
            cwd,
            timeout,
            argv,
        } => {
            let response = authorized(
                &client,
                Method::POST,
                format!("{base}/v1/sandboxes/{sandbox_id}/commands"),
                &token,
            )
            .json(&json!({
                "argv": argv,
                "cwd": cwd,
                "environment": {},
                "timeout_seconds": timeout,
                "max_output_bytes": 1048576
            }))
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        Command::Write {
            sandbox_id,
            remote_path,
            local_path,
            token,
            overwrite,
        } => {
            let data = tokio::fs::read(&local_path)
                .await
                .with_context(|| format!("read {}", local_path.display()))?;
            let response = authorized(
                &client,
                Method::PUT,
                format!("{base}/v1/sandboxes/{sandbox_id}/files"),
                &token,
            )
            .json(&json!({
                "path": remote_path,
                "content_base64": STANDARD.encode(data),
                "overwrite": overwrite
            }))
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        Command::Read {
            sandbox_id,
            remote_path,
            token,
            output,
        } => {
            let response = authorized(
                &client,
                Method::GET,
                format!("{base}/v1/sandboxes/{sandbox_id}/files"),
                &token,
            )
            .query(&[("path", remote_path)])
            .send()
            .await?;
            let value: Value = ensure_success(response).await?.json().await?;
            let encoded = value["content_base64"]
                .as_str()
                .context("response omitted content_base64")?;
            let data = STANDARD.decode(encoded)?;
            if let Some(path) = output {
                tokio::fs::write(&path, data)
                    .await
                    .with_context(|| format!("write {}", path.display()))?;
            } else {
                print!("{}", String::from_utf8_lossy(&data));
            }
        }
        Command::List {
            sandbox_id,
            remote_path,
            token,
        } => {
            let response = authorized(
                &client,
                Method::GET,
                format!("{base}/v1/sandboxes/{sandbox_id}/directories"),
                &token,
            )
            .query(&[("path", remote_path)])
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        Command::Pause { sandbox_id, token } => {
            let response = authorized(
                &client,
                Method::POST,
                format!("{base}/v1/sandboxes/{sandbox_id}/pause"),
                &token,
            )
            .send()
            .await?;
            ensure_success(response).await?;
            println!("paused {sandbox_id}");
        }
        Command::Resume { sandbox_id, token } => {
            let response = authorized(
                &client,
                Method::POST,
                format!("{base}/v1/sandboxes/{sandbox_id}/resume"),
                &token,
            )
            .send()
            .await?;
            ensure_success(response).await?;
            println!("resumed {sandbox_id}");
        }
        Command::Snapshot { command } => {
            handle_snapshot(&client, base, command).await?;
        }
        Command::Delete { sandbox_id, token } => {
            let response = authorized(
                &client,
                Method::DELETE,
                format!("{base}/v1/sandboxes/{sandbox_id}"),
                &token,
            )
            .send()
            .await?;
            ensure_success(response).await?;
            println!("deleted {sandbox_id}");
        }
    }
    Ok(())
}

async fn handle_snapshot(
    client: &Client,
    base: &str,
    command: SnapshotCommand,
) -> anyhow::Result<()> {
    match command {
        SnapshotCommand::Create {
            sandbox_id,
            name,
            token,
        } => {
            let response = authorized(
                client,
                Method::POST,
                format!("{base}/v1/sandboxes/{sandbox_id}/snapshots"),
                &token,
            )
            .json(&json!({ "name": name }))
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        SnapshotCommand::List {
            sandbox_id,
            limit,
            cursor,
            token,
        } => {
            let mut query = vec![("limit", limit.to_string())];
            if let Some(cursor) = cursor {
                query.push(("cursor", cursor));
            }
            let response = authorized(
                client,
                Method::GET,
                format!("{base}/v1/sandboxes/{sandbox_id}/snapshots"),
                &token,
            )
            .query(&query)
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        SnapshotCommand::Inspect { snapshot_id, token } => {
            let response = authorized(
                client,
                Method::GET,
                format!("{base}/v1/snapshots/{snapshot_id}"),
                &token,
            )
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        SnapshotCommand::Verify { snapshot_id, token } => {
            let response = authorized(
                client,
                Method::POST,
                format!("{base}/v1/snapshots/{snapshot_id}/verify"),
                &token,
            )
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        SnapshotCommand::Restore {
            snapshot_id,
            ttl,
            token,
        } => {
            let response = authorized(
                client,
                Method::POST,
                format!("{base}/v1/snapshots/{snapshot_id}/restore"),
                &token,
            )
            .json(&json!({ "timeout_seconds": ttl }))
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        SnapshotCommand::Clone {
            snapshot_id,
            count,
            ttl,
            token,
        } => {
            let response = authorized(
                client,
                Method::POST,
                format!("{base}/v1/snapshots/{snapshot_id}/clones"),
                &token,
            )
            .json(&json!({
                "count": count,
                "timeout_seconds": ttl
            }))
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        SnapshotCommand::Rollback {
            sandbox_id,
            snapshot_id,
            token,
        } => {
            let response = authorized(
                client,
                Method::POST,
                format!("{base}/v1/sandboxes/{sandbox_id}/rollback/{snapshot_id}"),
                &token,
            )
            .send()
            .await?;
            print_json(ensure_success(response).await?).await?;
        }
        SnapshotCommand::Delete { snapshot_id, token } => {
            let response = authorized(
                client,
                Method::DELETE,
                format!("{base}/v1/snapshots/{snapshot_id}"),
                &token,
            )
            .send()
            .await?;
            ensure_success(response).await?;
            println!("deleted snapshot {snapshot_id}");
        }
    }
    Ok(())
}

fn authorized(
    client: &Client,
    method: Method,
    url: String,
    token: &str,
) -> reqwest::RequestBuilder {
    client.request(method, url).bearer_auth(token)
}

async fn ensure_success(response: Response) -> anyhow::Result<Response> {
    let status = response.status();
    if status.is_success() {
        Ok(response)
    } else {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("API returned {status}: {body}")
    }
}

async fn print_json(response: Response) -> anyhow::Result<()> {
    let value: Value = response.json().await?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
