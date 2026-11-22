use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PennylaneError {
    #[error("could not reach Pennylane: {0}")]
    Transport(String),
    #[error("Pennylane answered {status}: {message}")]
    Api {
        status: u16,
        message: String,
        retry_after: Option<Duration>,
    },
    #[error("unexpected Pennylane response: {0}")]
    Decode(String),
}

impl PennylaneError {
    pub fn status(&self) -> Option<u16> {
        match self {
            PennylaneError::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Worth sending again as is: the request never reached Pennylane, was rate limited, hit a
    /// server error, conflicted with a state that settles on its own (a file still processing),
    /// or went through with an unreadable answer, which the next attempt's lookup recovers.
    pub fn is_retryable(&self) -> bool {
        match self {
            PennylaneError::Transport(_) | PennylaneError::Decode(_) => true,
            PennylaneError::Api { status, .. } => matches!(status, 409 | 429) || *status >= 500,
        }
    }

    /// The token was refused or lacks a scope: no request goes through until the company is
    /// reconnected.
    pub fn is_unauthorized(&self) -> bool {
        matches!(self.status(), Some(401 | 403))
    }

    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404)
    }

    /// What Pennylane said, without the transport noise, for showing to the merchant.
    pub fn message(&self) -> &str {
        match self {
            PennylaneError::Transport(message)
            | PennylaneError::Api { message, .. }
            | PennylaneError::Decode(message) => message,
        }
    }

    pub(crate) fn from_response(status: u16, body: &[u8], retry_after: Option<Duration>) -> Self {
        PennylaneError::Api {
            status,
            message: error_message(body),
            retry_after,
        }
    }
}

const MAX_MESSAGE_LEN: usize = 500;

fn error_message(body: &[u8]) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_slice(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| {
            ["error", "message", "errors"]
                .iter()
                .find_map(|key| value.get(key))
        })
        .map(|value| match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());

    let message = message.trim();
    match message.char_indices().nth(MAX_MESSAGE_LEN) {
        Some((cut, _)) => format!("{}…", &message[..cut]),
        None => message.to_owned(),
    }
}

impl From<reqwest::Error> for PennylaneError {
    fn from(err: reqwest::Error) -> PennylaneError {
        PennylaneError::Transport(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_is_read_from_the_error_body() {
        assert_eq!(
            error_message(br#"{"error":"Entry lines are not balanced","status":422}"#),
            "Entry lines are not balanced"
        );
        assert_eq!(
            error_message(br#"{"message":"Invalid vat_rate"}"#),
            "Invalid vat_rate"
        );
        assert_eq!(
            error_message(br#"{"errors":{"vat_rate":["is invalid"]}}"#),
            r#"{"vat_rate":["is invalid"]}"#
        );
        assert_eq!(
            error_message(b"Rate limit exceeded."),
            "Rate limit exceeded."
        );
    }

    #[test]
    fn a_long_body_is_cut() {
        let body = "x".repeat(2 * MAX_MESSAGE_LEN);
        assert_eq!(
            error_message(body.as_bytes()).chars().count(),
            MAX_MESSAGE_LEN + 1
        );
    }

    #[test]
    fn retryable_statuses() {
        let api = |status| PennylaneError::Api {
            status,
            message: String::new(),
            retry_after: None,
        };
        assert!(api(429).is_retryable());
        assert!(api(503).is_retryable());
        assert!(api(409).is_retryable());
        assert!(!api(422).is_retryable());
        assert!(!api(400).is_retryable());
        assert!(api(401).is_unauthorized());
        assert!(api(403).is_unauthorized());
    }
}
