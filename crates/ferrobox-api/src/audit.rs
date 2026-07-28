use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{
    fs::{self, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

#[derive(Debug, Clone)]
pub(crate) struct AuditLog {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Serialize)]
struct AuditEvent<'a> {
    timestamp_unix_ms: u128,
    sandbox_id: Option<&'a str>,
    operation: &'a str,
    outcome: &'a str,
    details: &'a BTreeMap<String, String>,
}

impl AuditLog {
    pub(crate) async fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        Ok(Self {
            path,
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    pub(crate) async fn record(
        &self,
        sandbox_id: Option<&str>,
        operation: &str,
        outcome: &str,
        details: &BTreeMap<String, String>,
    ) -> io::Result<()> {
        let timestamp_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let event = AuditEvent {
            timestamp_unix_ms,
            sandbox_id,
            operation,
            outcome,
            details,
        };
        let mut line = serde_json::to_vec(&event).map_err(io::Error::other)?;
        line.push(b'\n');

        let _guard = self.write_lock.lock().await;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(&line).await?;
        file.flush().await
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tempfile::tempdir;

    use super::AuditLog;

    #[tokio::test]
    async fn writes_one_json_object_per_line_without_secrets() {
        let directory = tempdir().expect("temporary directory");
        let log = AuditLog::open(directory.path().join("audit/events.jsonl"))
            .await
            .expect("open audit log");
        let mut details = BTreeMap::new();
        details.insert("state".to_owned(), "running".to_owned());

        log.record(Some("sandbox-1"), "create", "ok", &details)
            .await
            .expect("record event");

        let contents = tokio::fs::read_to_string(log.path())
            .await
            .expect("read audit log");
        let event: serde_json::Value =
            serde_json::from_str(contents.trim()).expect("valid JSON event");
        assert_eq!(event["sandbox_id"], "sandbox-1");
        assert_eq!(event["operation"], "create");
        assert!(event.get("token").is_none());
    }
}
