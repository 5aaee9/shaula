//! Operator diagnostics, separate from the stricter Runner Setup Info projection.
use std::sync::LazyLock;

use regex::Regex;

#[derive(Default)]
pub(super) struct Diagnostics {
    remaining: usize,
    pem: bool,
    dump_depth: usize,
    dump_quote: bool,
    dump_escape: bool,
    dump_paragraph: bool,
}

impl Diagnostics {
    pub(super) fn line(&mut self, text: &str) -> Option<String> {
        let content = text.trim_start().trim_start_matches('│').trim_start();
        if self.pem || content.contains("-----BEGIN ") {
            self.pem = !content.contains("-----END ");
            return None;
        }
        if self.dump_depth > 0 || content.contains('{') || structured_array(content) {
            self.track_dump(content);
            return None;
        }
        if self.dump_paragraph {
            self.dump_paragraph = !content.is_empty();
            return None;
        }
        let lowered = content.to_ascii_lowercase();
        if [
            "response body:",
            "request body:",
            "state dump:",
            "debug dump:",
        ]
        .iter()
        .any(|marker| lowered.contains(marker))
        {
            self.dump_paragraph = true;
            return None;
        }
        let text = redact_url_parts(text);
        let content = text.trim_start().trim_start_matches('│').trim_start();
        if unsafe_diagnostic(content) {
            return None;
        }
        if content.starts_with("Error:") || content.starts_with("Warning:") {
            // A diagnostic may have paragraphs and source context, but cannot
            // authorize an unbounded stream of arbitrary provider output.
            self.remaining = 256;
            return Some(text);
        }
        if let Some(progress) = super::progress_line(content) {
            self.remaining = 0;
            return Some(progress);
        }
        if provider_context(content) {
            return Some(text);
        }
        if content == "╷" || content == "╵" {
            self.remaining = 0;
            return Some(text);
        }
        if self.remaining > 0 {
            self.remaining -= 1;
            return Some(text);
        }
        None
    }

    fn track_dump(&mut self, text: &str) {
        for character in text.chars() {
            if self.dump_escape {
                self.dump_escape = false;
            } else if self.dump_quote && character == '\\' {
                self.dump_escape = true;
            } else if character == '"' {
                self.dump_quote = !self.dump_quote;
            } else if !self.dump_quote {
                match character {
                    '{' | '[' => self.dump_depth = self.dump_depth.saturating_add(1),
                    '}' | ']' => self.dump_depth = self.dump_depth.saturating_sub(1),
                    _ => {}
                }
            }
        }
    }
}

fn structured_array(text: &str) -> bool {
    text.starts_with('[') && !text.starts_with("[REDACTED]")
}

fn unsafe_diagnostic(text: &str) -> bool {
    static ASSIGNMENT: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r#"(?i)(?:password|passwd|token|secret|credential|authorization|cookie|private[_ -]?key|client[_ -]?secret|access[_ -]?key)\s*(?:[:=]|\bis\b)\s*\S|^\s*(?:\d+:\s*)?[\w.\[\]"-]+\s*=|^\s*"[^"\n]+"\s*:"#).ok()
    });
    static SECRET: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r"(?i)\bbearer\s+\S+|\b(?:gh[pousr]_|github_pat_|AKIA|ASIA)[a-z0-9_]+|\beyJ[a-z0-9_-]+\.[a-z0-9_-]+\.[a-z0-9_-]+|[a-z0-9+/_=-]{80,}|\b[a-f0-9]{32,}\b").ok()
    });
    ASSIGNMENT.as_ref().is_some_and(|re| re.is_match(text))
        || SECRET.as_ref().is_some_and(|re| re.is_match(text))
}

fn provider_context(text: &str) -> bool {
    static PROVIDER: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(r#"^- (?:Finding|Installing|Installed|Using previously-installed|Reusing previous version of) [a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+(?: from the dependency lock file| versions matching "[0-9.,~><= -]+"| v[0-9][0-9a-zA-Z.+-]*)?(?:\.\.\.| \((?:signed by HashiCorp|unauthenticated|self-signed, key ID [A-Fa-f0-9]{8,40})\))?$"#).ok()
    });
    PROVIDER.as_ref().is_some_and(|re| re.is_match(text))
}

fn redact_url_parts(text: &str) -> String {
    static URL: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r#"https?://[^\s<>"']+"#).ok());
    let Some(url) = URL.as_ref() else {
        return "[REDACTED URL]".into();
    };
    url.replace_all(text, |capture: &regex::Captures<'_>| {
        let original = &capture[0];
        let Ok(mut url) = url::Url::parse(original) else {
            return "[REDACTED URL]".into();
        };
        let credentials = !url.username().is_empty() || url.password().is_some();
        let query = url.query().is_some();
        let fragment = url.fragment().is_some();
        if !credentials && !query && !fragment {
            return original.to_string();
        }
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        let mut projected = url.to_string();
        if credentials {
            projected.insert_str(url.scheme().len() + 3, "[REDACTED]@");
        }
        if query {
            projected.push_str("?[REDACTED]");
        }
        if fragment {
            projected.push_str("#[REDACTED]");
        }
        projected
    })
    .into_owned()
}
