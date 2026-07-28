use std::time::SystemTime;

use async_trait::async_trait;

/// A lifecycle or data-plane operation recorded by the node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    pub sandbox_id: String,
    pub action: AuditAction,
    pub outcome: AuditOutcome,
    pub occurred_at: SystemTime,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditAction {
    Create,
    Execute,
    Signal,
    WriteFile,
    ReadFile,
    ListDirectory,
    Pause,
    Resume,
    Delete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditOutcome {
    Started,
    Succeeded,
    Failed,
}

#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn record(&self, event: AuditEvent);
}

#[derive(Debug, Default)]
pub struct TracingAuditSink;

#[async_trait]
impl AuditSink for TracingAuditSink {
    async fn record(&self, event: AuditEvent) {
        tracing::info!(
            target: "ferrobox.audit",
            sandbox_id = %event.sandbox_id,
            action = ?event.action,
            outcome = ?event.outcome,
            detail = event.detail.as_deref().unwrap_or_default(),
            "sandbox audit event"
        );
    }
}
