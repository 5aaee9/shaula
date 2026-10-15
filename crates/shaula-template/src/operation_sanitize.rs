//! Record-buffered publication: provider output is published by default with
//! known values and sensitive shapes redacted in place. Records that cannot be
//! redacted safely (invalid encoding, oversized, control characters, workflow
//! commands, key material) are withheld instead of ever falling back to raw.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Deserialize;
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
        // All string input bindings/parameters are treated conservatively as sensitive.
        if let Some(input) = value.get("shaula").or(Some(value)) {
            for key in [
                "jit_config",
                "bindings",
                "parameters",
                "setup_info",
                "forgejo_vm",
                "gitlab_vm",
            ] {
                if let Some(value) = input.get(key) {
                    values.collect(value, 0);
                }
            }
            // config.toml is collected per line above; the bare Runner token
            // can also be echoed on its own.
            if let Some(Ok(vm)) = input
                .get("gitlab_vm")
                .map(shaula_core::template::GitlabVmBootstrap::deserialize)
            {
                values.add(vm.token());
            }
        }
        for (name, value) in env {
            if !crate::http_backend::PUBLIC_SETTINGS.contains(&name.as_str()) {
                values.add(value);
            }
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
            // Numbers are sizing/IDs, never credentials; redacting `2` in place
            // erased every digit of plan output, IDs and digests. Credentials
            // are strings and stay conservatively redacted above.
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
            || text.contains("##[")
            || diagnostics::actions_command(text)
        {
            return (WITHHELD.into(), true);
        }
        match self.diagnostics.line(text) {
            Some(line) => (format!("{line}\n"), false),
            None => (WITHHELD.into(), true),
        }
    }
}

#[cfg(test)]
#[path = "operation_sanitize_diagnostic_tests.rs"]
mod diagnostic_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vm_cloud_init_token_is_redacted_on_create_and_retained_destroy_input(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let forgejo = "vm-secret-\"quoted\"-canary";
        let gitlab = "glrt-vm-secret-canary";
        let material = shaula_core::ports::gitlab::GitlabBootstrapMaterial::new(
            "https://gitlab.test",
            7,
            "runner",
            shaula_core::secret::SecretString::new(gitlab),
        )?;
        let vm = shaula_core::template::GitlabVmBootstrap::from_registration(&material);
        // Create passes the envelope; Destroy reloads the protected tfvars document.
        for (token, input) in [
            (forgejo, serde_json::json!({"forgejo_vm":{"token":forgejo}})),
            (gitlab, serde_json::json!({ "gitlab_vm": vm })),
        ] {
            for input in [input.clone(), serde_json::json!({ "shaula": input })] {
                let values = SensitiveValues::from_input(&input, &[]);
                let mut sanitizer = Sanitizer::new(std::sync::Arc::new(values));
                for secret in [
                    token.into(),
                    STANDARD.encode(token),
                    hex::encode(token),
                    serde_json::to_string(token)?.trim_matches('"').to_string(),
                ] {
                    let record = format!("Error: provider echoed {secret}\n");
                    let output = sanitizer.feed(record.as_bytes());
                    assert_eq!(output[0].0, "Error: provider echoed [REDACTED]\n");
                }
            }
        }
        Ok(())
    }

    // molecule's Proxmox profile: numeric sizing and the HTTP backend's fixed
    // retry/method settings made every `1`/`2`/`5` in plan output, provider
    // versions, resource IDs and ISO digests read `[REDACTED]`.
    #[test]
    fn numeric_inputs_and_public_backend_settings_are_not_redacted() {
        let token = "root@pam!shaula=0b9c2a1e-canary";
        let input = serde_json::json!({"shaula": {
            "bindings": {"proxmox_token": token, "proxmox_vmid_begin": 9000},
            "parameters": {"cpu_cores": 2, "memory_mb": 4096, "threads": "8"},
            "jit_config": STANDARD.encode(r#"{"runner_id":1,"secret":"jit-canary"}"#),
        }});
        let address = "http://127.0.0.1:9/state/address-canary";
        let mut env: Vec<(String, String)> = [
            ("TF_HTTP_ADDRESS", address),
            ("TF_HTTP_LOCK_ADDRESS", address),
            ("TF_HTTP_UNLOCK_ADDRESS", address),
            ("TF_HTTP_UPDATE_METHOD", "POST"),
            ("TF_HTTP_LOCK_METHOD", "LOCK"),
            ("TF_HTTP_UNLOCK_METHOD", "UNLOCK"),
            ("TF_HTTP_USERNAME", "shaula-state"),
            ("TF_HTTP_PASSWORD", "password-canary"),
            ("TF_HTTP_RETRY_MAX", "2"),
            ("TF_HTTP_RETRY_WAIT_MIN", "1"),
            ("TF_HTTP_RETRY_WAIT_MAX", "5"),
        ]
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .into();
        // A provider-supplied variable is not a public setting even when tiny.
        env.push(("PROVIDER_PIN".into(), "pin-canary".into()));
        let mut sanitizer = Sanitizer::new(std::sync::Arc::new(SensitiveValues::from_input(
            &input, &env,
        )));
        for line in [
            "proxmox_qemu_vm.runner: Creation complete after 2s [id=tyo-01-generic/qemu/9001]\n",
            "Plan: 0 to add, 0 to change, 2 to destroy.\n",
            "- Installed indexyz/proxmox v0.5.1 (self-signed, key ID BC71377E49D4FF7E)\n",
            "Releasing state lock (POST/LOCK/UNLOCK as shaula-state). This may take a few moments...\n",
        ] {
            assert_eq!(sanitizer.feed(line.as_bytes())[0].0, line);
        }
        for secret in [
            token.to_string(),
            STANDARD.encode(token),
            hex::encode(token),
            "jit-canary".into(),
            "threads 8".into(),
            "password-canary".into(),
            address.into(),
            "pin-canary".into(),
        ] {
            let output = sanitizer.feed(format!("Error: echoed {secret}\n").as_bytes());
            assert!(output[0].0.contains("[REDACTED]"), "{secret}");
            assert!(!output[0].0.contains("canary"), "{secret}");
        }
    }

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
            "password = [REDACTED]\n"
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
            "docker_container.runner: Creation complete after 5s [id=private-id]\n"
        );
        assert_eq!(
            sanitizer.feed(b"Apply complete! Resources: 1 added, 0 changed, 0 destroyed.\n")[0].0,
            "Apply complete! Resources: 1 added, 0 changed, 0 destroyed.\n"
        );
    }

    #[test]
    fn exhausted_budget_withholds_and_known_values_are_redacted_in_place() {
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
            "[REDACTED]: Creating...\n"
        );
    }
}
