use serde_json::Value;

use super::AuditAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Denied,
    Failure,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Denied => "denied",
            Self::Failure => "failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub kind: &'static str,
    pub id: String,
}

/// Ce qui s'est passé. Les détails ne contiennent jamais de secret (mot de passe, token,
/// clé) ni de donnée personnelle superflue.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub action: AuditAction,
    pub outcome: Outcome,
    pub target: Option<Target>,
    pub details: Value,
}

impl AuditEvent {
    pub fn success(action: AuditAction) -> Self {
        Self::new(action, Outcome::Success)
    }

    pub fn denied(action: AuditAction) -> Self {
        Self::new(action, Outcome::Denied)
    }

    pub fn failure(action: AuditAction) -> Self {
        Self::new(action, Outcome::Failure)
    }

    fn new(action: AuditAction, outcome: Outcome) -> Self {
        Self {
            action,
            outcome,
            target: None,
            details: Value::Object(serde_json::Map::new()),
        }
    }

    pub fn target(mut self, kind: &'static str, id: impl ToString) -> Self {
        self.target = Some(Target {
            kind,
            id: id.to_string(),
        });
        self
    }

    pub fn details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }
}
