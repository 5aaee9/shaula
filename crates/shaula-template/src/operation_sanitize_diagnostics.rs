//! Operator diagnostics: publish by default, redact sensitive content in place.
//!
//! The caller replaces every known sensitive value before this stage. Here we
//! redact sensitive-keyed assignments, JSON pairs and well-known token shapes
//! so ordinary provider text survives. Only content that cannot be redacted
//! in place (PEM blocks) is withheld; redaction failures fail closed.
use std::sync::LazyLock;

use regex::Regex;

/// Key-name fragments whose values are never published, in any syntax.
const SENSITIVE_KEYS: &str = r"(?:password|passwd|token|secret|credential|authorization|cookie|private[_ -]?key|client[_ -]?secret|access[_ -]?key|api[_ -]?key)";

#[derive(Default)]
pub(super) struct Diagnostics {
    pem: bool,
}

impl Diagnostics {
    /// Returns the publishable line; `None` when the record must be withheld.
    pub(super) fn line(&mut self, text: &str) -> Option<String> {
        let content = text.trim_start().trim_start_matches('│').trim_start();
        if self.pem || content.contains("-----BEGIN ") {
            self.pem = !content.contains("-----END ");
            return None;
        }
        redact_inline(text)
    }
}

/// GitHub Actions workflow commands (`::error ...::`, `::add-mask::`) never
/// leave the sanitizer: archived text may later be echoed into a workflow
/// step where the runner would execute them. Plain `::` (IPv6, namespaces)
/// is not matched.
pub(super) fn actions_command(text: &str) -> bool {
    static COMMAND: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r"::(?:add-mask|set-output|set-env|add-path|add-matcher|remove-matcher|error|warning|notice|debug|group|endgroup|save-state|stop-commands|echo)(?:[ \t][^\n:]*)?::",
        )
        .ok()
    });
    COMMAND.as_ref().is_some_and(|re| re.is_match(text))
}

fn redact_inline(text: &str) -> Option<String> {
    let text = redact_url_parts(text);
    let text = redact_json_pairs(&text)?;
    let text = redact_assignments(&text)?;
    redact_tokens(&text)
}

/// `"key": "value"` pairs with a sensitive key keep the key; the value is
/// redacted, including quoted values that contain spaces.
fn redact_json_pairs(text: &str) -> Option<String> {
    static PAIR: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(&format!(
            r#"(?i)("[^"\n]*{keys}[^"\n]*")(\s*:\s*)("(?:[^"\\\n]|\\.)*"|-?[0-9]+(?:\.[0-9]+)?|true|false|null)"#,
            keys = SENSITIVE_KEYS,
        ))
        .ok()
    });
    let pair = PAIR.as_ref()?;
    Some(
        pair.replace_all(text, r#"${1}${2}"[REDACTED]""#)
            .into_owned(),
    )
}

/// Sensitive assignments (`password = x`, `token: x`, `secret is x`) keep the
/// key; the value and the rest of the record are redacted so headers such as
/// `Authorization: Bearer …` cannot leak their second token.
fn redact_assignments(text: &str) -> Option<String> {
    static ASSIGNMENT: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(&format!(
            r#"(?i)({keys}[\w.\[\]" -]*\s*(?:[:=]|\bis\b)\s*)\S[^\n]*"#,
            keys = SENSITIVE_KEYS,
        ))
        .ok()
    });
    let assignment = ASSIGNMENT.as_ref()?;
    Some(assignment.replace_all(text, "${1}[REDACTED]").into_owned())
}

/// Well-known token shapes are redacted wherever they appear in a line.
fn redact_tokens(text: &str) -> Option<String> {
    static BEARER: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"\b([Bb]earer[ \t]+)\S+").ok());
    static WELL_KNOWN: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r"\b(?:gh[pousr]_|github_pat_|AKIA|ASIA)[A-Za-z0-9_]+|\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
        )
        .ok()
    });
    static LONG_BLOB: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"[A-Za-z0-9+/_=-]{80,}").ok());
    static LONG_HEX: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"\b[a-f0-9]{32,}\b").ok());
    let text = BEARER.as_ref()?.replace_all(text, "${1}[REDACTED]");
    let text = WELL_KNOWN.as_ref()?.replace_all(&text, "[REDACTED]");
    let text = LONG_BLOB.as_ref()?.replace_all(&text, "[REDACTED]");
    let text = LONG_HEX.as_ref()?.replace_all(&text, "[REDACTED]");
    Some(text.into_owned())
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
