use std::{env, error::Error, fs};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ferrobox_client::{
    apis::{
        commands_api, configuration::Configuration, files_api, sandboxes_api, Error as ApiError,
    },
    models,
};
use serde_json::json;

const CHECKS: [&str; 7] = [
    "generated-model-create",
    "bearer-auth-inspect",
    "typed-command-execution",
    "lossless-base64-output",
    "typed-file-roundtrip",
    "delete-and-stale-handle-rejection",
    "credential-redaction",
];

fn required_environment(name: &str) -> Result<String, Box<dyn Error>> {
    env::var(name).map_err(|_| format!("{name} is required").into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let api_url = required_environment("FERROBOX_API_URL")?;
    let audit_path = required_environment("FERROBOX_AUDIT_LOG")?;
    let evidence_path = required_environment("FERROBOX_OPENAPI_SDK_EVIDENCE")?;

    let mut public_configuration = Configuration::new();
    public_configuration.base_path = api_url.clone();
    let mut create_request = models::CreateSandboxRequest::new("python".to_owned());
    create_request.cpu_count = Some(1);
    create_request.memory_mb = Some(512);
    create_request.timeout_seconds = Some(120);
    let mut network = models::NetworkRequest::new();
    network.internet_access = Some(false);
    create_request.network = Some(Box::new(network));
    let created = sandboxes_api::create_sandbox(&public_configuration, create_request).await?;
    if created.state != models::SandboxState::Running || created.token.is_empty() {
        return Err("created sandbox identity is invalid".into());
    }

    let sandbox_id = created.sandbox_id.to_string();
    let token = created.token;
    let mut configuration = Configuration::new();
    configuration.base_path = api_url;
    configuration.bearer_access_token = Some(token.clone());

    let inspected = sandboxes_api::get_sandbox(&configuration, &sandbox_id).await?;
    if inspected.sandbox_id != created.sandbox_id
        || inspected.state != models::SandboxState::Running
    {
        return Err("inspect returned invalid sandbox state".into());
    }

    let mut execute_request = models::ExecuteCommandRequest::new(vec![
        "python3".to_owned(),
        "-c".to_owned(),
        "print(40 + 2)".to_owned(),
    ]);
    execute_request.cwd = Some("/home/sandbox".to_owned());
    execute_request.environment = Some(Default::default());
    execute_request.timeout_seconds = Some(30);
    execute_request.max_output_bytes = Some(1_048_576);
    let executed =
        commands_api::execute_command(&configuration, &sandbox_id, execute_request).await?;
    if executed.stdout != "42\n" || BASE64.decode(&executed.stdout_base64)? != b"42\n" {
        return Err("command output mismatch".into());
    }

    let payload = b"generated-openapi-client\n";
    let mut write_request = models::WriteFileRequest::new(
        "/home/sandbox/openapi.txt".to_owned(),
        BASE64.encode(payload),
    );
    write_request.overwrite = Some(false);
    let written = files_api::write_file(&configuration, &sandbox_id, write_request).await?;
    if written.bytes_written != payload.len() as i64 {
        return Err("written byte count mismatch".into());
    }

    let read = files_api::read_file(
        &configuration,
        &sandbox_id,
        "/home/sandbox/openapi.txt",
        Some(0),
        Some(1_048_576),
    )
    .await?;
    if BASE64.decode(&read.content_base64)? != payload || !read.eof {
        return Err("file roundtrip mismatch".into());
    }

    sandboxes_api::delete_sandbox(&configuration, &sandbox_id).await?;
    let stale = sandboxes_api::get_sandbox(&configuration, &sandbox_id).await;
    match stale {
        Err(ApiError::ResponseError(response)) if response.status.as_u16() == 404 => {}
        _ => return Err("deleted sandbox remained addressable".into()),
    }

    let audit = fs::read_to_string(audit_path)?;
    if audit.contains(&token) || !audit.contains("\"operation\":\"delete\"") {
        return Err("audit credential-redaction check failed".into());
    }

    let evidence = json!({
        "schema_version": 1,
        "language": "rust",
        "sandbox_id": sandbox_id,
        "checks": CHECKS,
    });
    let serialized = serde_json::to_string_pretty(&evidence)? + "\n";
    fs::write(evidence_path, &serialized)?;
    print!("{serialized}");
    Ok(())
}
