//! Record-buffered publication: arbitrary provider dumps never become log text.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use zeroize::Zeroizing;

const RECORD_LIMIT: usize = 64 * 1024;
pub(crate) const WITHHELD: &str = "[Shaula: diagnostic withheld by publication policy]\n";

#[path = "operation_sanitize_diagnostics.rs"]
mod diagnostics;

#[derive(Default)]
pub(crate) struct SensitiveValues {
    values: Vec<Zeroizing<String>>,
    bytes: usize,
    exhausted: bool,
}

impl SensitiveValues {
    pub(crate) fn withhold_all() -> Self {
        Self {
            exhausted: true,
            ..Self::default()
        }
    }

    pub(crate) fn from_input(value: &Value, env: &[(String, String)]) -> Self {
        let mut values = Self::default();
        // All input bindings/parameters are treated conservatively as sensitive.
        if let Some(input) = value.get("shaula").or(Some(value)) {
            for key in ["jit_config", "bindings", "parameters", "setup_info"] {
                if let Some(value) = input.get(key) {
                    values.collect(value, 0);
                }
            }
        }
        for (_, value) in env {
            values.add(value);
        }
        values.values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        values.values.dedup_by(|a, b| a.as_str() == b.as_str());
        values
    }

    fn collect(&mut self, value: &Value, depth: usize) {
        if depth > 12 || self.exhausted {
            self.exhausted = true;
            return;
        }
        match value {
            Value::String(text) => {
                self.add(text);
                if text.len() < 1024 * 1024 {
                    if let Ok(decoded) = STANDARD.decode(text) {
                        let decoded = Zeroizing::new(decoded);
                        if let Ok(text) = std::str::from_utf8(&decoded) {
                            self.add(text);
                            if let Ok(value) = serde_json::from_str(text) {
                                self.collect(&value, depth + 1);
                            }
                        }
                    }
                    if let Ok(nested) = serde_json::from_str::<Value>(text) {
                        if !matches!(nested, Value::String(_)) {
                            self.collect(&nested, depth + 1);
                        }
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.collect(value, depth + 1);
                }
            }
            Value::Object(values) => {
                for value in values.values() {
                    self.collect(value, depth + 1);
                }
            }
            Value::Number(value) => self.add(&value.to_string()),
            _ => {}
        }
    }

    fn add(&mut self, text: &str) {
        if text.is_empty() || self.exhausted {
            return;
        }
        if text.len() > 1024 * 1024 {
            self.exhausted = true;
            return;
        }
        for line in text.lines().filter(|line| !line.is_empty()) {
            if line.len() <= RECORD_LIMIT {
                self.insert(line.to_string());
                self.insert(STANDARD.encode(line));
                self.insert(hex::encode(line));
                if let Ok(escaped) = serde_json::to_string(line) {
                    self.insert(escaped.trim_matches('"').to_string());
                }
            } else {
                self.exhausted = true;
            }
            if self.exhausted {
                break;
            }
        }
    }

    fn insert(&mut self, value: String) {
        let value = Zeroizing::new(value);
        if self.values.len() >= 4096 || self.bytes.saturating_add(value.len()) > 4 * 1024 * 1024 {
            self.exhausted = true;
        } else if !self.exhausted {
            self.bytes += value.len();
            self.values.push(value);
        }
    }
}

pub(crate) struct Sanitizer {
    pending: Zeroizing<Vec<u8>>,
    dropping: bool,
    values: std::sync::Arc<SensitiveValues>,
    diagnostics: diagnostics::Diagnostics,
}

impl Sanitizer {
    pub(crate) fn new(values: std::sync::Arc<SensitiveValues>) -> Self {
        Self {
            pending: Zeroizing::new(Vec::new()),
            dropping: false,
            values,
            diagnostics: diagnostics::Diagnostics::default(),
        }
    }

    /// Only complete records leave the sanitizer. The caller may then drop them.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<(String, bool)> {
        let mut result = Vec::new();
        for byte in bytes {
            if self.dropping {
                if *byte == b'\n' {
                    self.dropping = false;
                }
                continue;
            }
            self.pending.push(*byte);
            if self.pending.len() > RECORD_LIMIT {
                self.pending.clear();
                self.dropping = *byte != b'\n';
                result.push((WITHHELD.into(), true));
            } else if *byte == b'\n' {
                result.push(self.record());
            }
        }
        result
    }

    pub(crate) fn finish(&mut self, complete: bool) -> Option<(String, bool)> {
        if self.pending.is_empty() {
            return None;
        }
        if complete {
            Some(self.record())
        } else {
            self.pending.clear();
            Some((WITHHELD.into(), true))
        }
    }

    fn record(&mut self) -> (String, bool) {
        if self.values.exhausted {
            self.pending.clear();
            return (WITHHELD.into(), true);
        }
        let text = std::str::from_utf8(&self.pending).ok().map(str::to_string);
        self.pending.clear();
        let Some(mut text) = text else {
            return (WITHHELD.into(), true);
        };
        for value in &self.values.values {
            text = text.replace(value.as_str(), "[REDACTED]");
        }
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            return ("\n".into(), false);
        }
        if text.chars().any(|c| c.is_control() && c != '\t')
            || text.contains("::")
            || text.contains("##[")
        {
            return (WITHHELD.into(), true);
        }
        match self.diagnostics.line(text) {
            Some(line) => (format!("{line}\n"), false),
            None => (WITHHELD.into(), true),
        }
    }
}

fn progress_line(text: &str) -> Option<String> {
    let text = text.trim();
    let static_lines = [
        "Initializing the backend...",
        "Initializing provider plugins...",
        "Terraform has been successfully initialized!",
        "No changes. Your infrastructure matches the configuration.",
        "Success! The configuration is valid.",
    ];
    if static_lines.contains(&text) {
        return Some(text.into());
    }
    for prefix in [
        "Apply complete! Resources:",
        "Destroy complete! Resources:",
        "Plan:",
    ] {
        if let Some(tail) = text.strip_prefix(prefix) {
            if tail
                .chars()
                .all(|c| c.is_ascii_digit() || " addedchangedestroytoimport.,- ".contains(c))
            {
                return Some(text.into());
            }
        }
    }
    if let Some((address, action)) = text.split_once(": ") {
        let safe_address = address.split('[').next().unwrap_or_default();
        if safe_address.len() <= 256
            && safe_address.contains('.')
            && safe_address
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        {
            for verb in [
                "Creating...",
                "Still creating...",
                "Creation complete",
                "Destroying...",
                "Still destroying...",
                "Destruction complete",
                "Refreshing state...",
                "Reading...",
                "Read complete",
            ] {
                if let Some(tail) = action.strip_prefix(verb) {
                    // Provider IDs, URLs and data-bearing suffixes are never projected.
                    let duration = tail.split('[').next().unwrap_or_default().trim();
                    if duration
                        .chars()
                        .all(|c| c.is_ascii_digit() || " after elapsedhms.,- ".contains(c))
                    {
                        return Some(
                            format!("{safe_address}: {verb} {duration}")
                                .trim_end()
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "operation_sanitize_diagnostic_tests.rs"]
mod diagnostic_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_secret_and_unsafe_provider_dump_are_never_released() {
        let values = SensitiveValues::from_input(&serde_json::json!({"jit_config":"abcdef"}), &[]);
        let mut sanitizer = Sanitizer::new(std::sync::Arc::new(values));
        assert!(sanitizer.feed(b"Error: abc").is_empty());
        let text = sanitizer
            .feed(b"def\n")
            .into_iter()
            .map(|v| v.0)
            .collect::<String>();
        assert!(!text.contains("abc"));
        assert!(!text.contains("def"));
        assert!(text.contains("Error:"));
        assert_eq!(
            sanitizer.feed(b"password = arbitrary-new-secret\n")[0].0,
            WITHHELD
        );
    }

    #[test]
    fn truncated_record_is_withheld_and_detailed_progress_survives() {
        let mut sanitizer = Sanitizer::new(std::sync::Arc::new(SensitiveValues::default()));
        sanitizer.feed(b"Error: half-secret");
        assert_eq!(sanitizer.finish(false).map(|v| v.0), Some(WITHHELD.into()));
        let line = sanitizer
            .feed(b"docker_container.runner: Creation complete after 5s [id=private-id]\n");
        assert_eq!(
            line[0].0,
            "docker_container.runner: Creation complete after 5s\n"
        );
        assert_eq!(
            sanitizer.feed(b"Apply complete! Resources: 1 added, 0 changed, 0 destroyed.\n")[0].0,
            "Apply complete! Resources: 1 added, 0 changed, 0 destroyed.\n"
        );
    }

    #[test]
    fn exhausted_secret_budget_withholds_even_otherwise_safe_progress() {
        let input = serde_json::json!({"bindings": {"large": "x".repeat(RECORD_LIMIT + 1)}});
        let mut sanitizer = Sanitizer::new(std::sync::Arc::new(SensitiveValues::from_input(
            &input,
            &[],
        )));
        assert_eq!(
            sanitizer.feed(b"Apply complete! Resources: 1 added, 0 changed, 0 destroyed.\n")[0].0,
            WITHHELD
        );
        let mut sanitizer = Sanitizer::new(std::sync::Arc::new(SensitiveValues::from_input(
            &serde_json::Value::Null,
            &[("TF_HTTP_PASSWORD".into(), "private.backend".into())],
        )));
        assert_eq!(
            sanitizer.feed(b"private.backend: Creating...\n")[0].0,
            WITHHELD
        );
    }
}
