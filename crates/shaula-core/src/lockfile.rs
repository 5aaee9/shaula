//! Terraform dependency-lock (`.terraform.lock.hcl`) parsing.
//!
//! The registry recomputes the attestation subject's provider set from
//! THIS file — the immutable lock shipped inside the admitted artifact —
//! never from a submitted claim (spec 0005 §5.1). The parser accepts the
//! fixed provider-block shape `terraform init` writes; anything else is a
//! parse error so a malformed lock can never yield an empty/lenient
//! subject.

use crate::error::{CoreError, CoreResult, ReasonCode};
use sha2::{Digest, Sha256};

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::new(ReasonCode::TemplateInvalid, msg)
}

/// One locked provider: registry source, pinned version and the
/// commitment over its full checksum list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedProvider {
    pub source: String,
    pub version: String,
    /// `sha256:` over the newline-joined, sorted hash list.
    pub checksums_digest: String,
}

/// Parses every `provider "<addr>" { ... }` block of a lock file. The
/// grammar is STRICT (R5-03/R6-10): any top-level line that is not
/// blank, a comment, a `terraform { ... }` settings block OPENED ON THE
/// SAME LINE (a bare `terraform` word is syntax garbage, not a block),
/// or a provider block is a parse error, so a garbage file can never
/// masquerade as an empty (provider-free) lock. Every declared provider
/// MUST pin a non-empty checksum list of SUPPORTED representations —
/// a version-only block would authenticate as SHA256(empty), and an
/// arbitrary junk checksum would still hash into a "valid" commitment
/// that proves nothing.
pub fn parse_lock_providers(lock_hcl: &str) -> CoreResult<Vec<LockedProvider>> {
    let mut providers = Vec::new();
    let mut lines = lock_hcl.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("terraform") && trimmed[9..].trim_start().starts_with('{') {
            // A terraform settings block carries no provider pins; skip
            // it with balanced-brace tracking so its content is never
            // mistaken for provider syntax.
            let mut depth =
                trimmed.matches('{').count() as i64 - trimmed.matches('}').count() as i64;
            if depth <= 0 {
                continue;
            }
            while depth > 0 {
                let Some(inner) = lines.next() else {
                    return Err(err("lock file terraform block is not closed"));
                };
                let inner = inner.trim();
                depth += inner.matches('{').count() as i64;
                depth -= inner.matches('}').count() as i64;
            }
            continue;
        }
        let parsed = trimmed
            .strip_prefix("provider")
            .map(str::trim)
            .and_then(|rest| rest.strip_prefix('"'))
            .and_then(|rest| rest.split_once('"'));
        let Some((address, tail)) = parsed else {
            return Err(err(format!(
                "lock file contains unsupported syntax at {trimmed:?}"
            )));
        };
        if tail.trim() != "{" {
            return Err(err("lock file provider block header malformed"));
        }
        let address = address.to_string();
        let mut version: Option<String> = None;
        let mut hashes: Vec<String> = Vec::new();
        let mut in_hashes = false;
        let mut closed = false;
        for inner in lines.by_ref() {
            let inner = inner.trim();
            if in_hashes {
                if inner.starts_with(']') {
                    in_hashes = false;
                    continue;
                }
                let hash = inner
                    .trim_end_matches(',')
                    .trim()
                    .strip_prefix('"')
                    .and_then(|h| h.strip_suffix('"'));
                match hash {
                    Some(h) => hashes.push(checksum_value(h, &address)?),
                    None => return Err(err("lock file hash list has a malformed entry")),
                }
                continue;
            }
            if inner.starts_with('}') {
                closed = true;
                break;
            }
            if let Some(rest) = inner.strip_prefix("version") {
                version = Some(hcl_string(rest, "version")?);
                continue;
            }
            if inner.starts_with("hashes") {
                let Some(open) = inner.find('[') else {
                    return Err(err("lock file hashes must be a list"));
                };
                let list = &inner[open + 1..];
                if let Some((inline, _)) = list.split_once(']') {
                    // Single-line form: hashes = ["h1:a", "zh:b"]
                    for entry in inline.split(',') {
                        let entry = entry.trim();
                        if entry.is_empty() {
                            continue;
                        }
                        let hash = entry.strip_prefix('"').and_then(|h| h.strip_suffix('"'));
                        match hash {
                            Some(h) => hashes.push(checksum_value(h, &address)?),
                            None => return Err(err("lock file hash list has a malformed entry")),
                        }
                    }
                    continue;
                }
                in_hashes = true;
                continue;
            }
            // constraints / other keys are accepted but not projected.
        }
        if !closed {
            return Err(err("lock file provider block is not closed"));
        }
        let version = version
            .ok_or_else(|| err(format!("lock file provider {address} has no version pin")))?;
        if hashes.is_empty() {
            return Err(err(format!(
                "lock file provider {address} declares no checksum pins"
            )));
        }
        hashes.sort();
        let mut hasher = Sha256::new();
        for hash in &hashes {
            hasher.update(hash.as_bytes());
            hasher.update(b"\n");
        }
        providers.push(LockedProvider {
            source: address,
            version,
            checksums_digest: format!("sha256:{}", hex::encode(hasher.finalize())),
        });
    }
    Ok(providers)
}

/// The subject's short provider source: the default registry prefix is
/// implied by convention (`registry.terraform.io/hashicorp/kubernetes` →
/// `hashicorp/kubernetes`); explicit registry hosts are kept verbatim.
pub fn provider_source(lock_address: &str) -> String {
    lock_address
        .strip_prefix("registry.terraform.io/")
        .unwrap_or(lock_address)
        .to_string()
}

fn hcl_string(rest: &str, key: &str) -> CoreResult<String> {
    let value = rest
        .trim()
        .strip_prefix('=')
        .map(str::trim)
        .unwrap_or(rest.trim())
        .trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .ok_or_else(|| err(format!("lock file {key} must be a quoted string")))?;
    Ok(value.to_string())
}

/// Validates ONE checksum pin (R6-10, R7-02, R8-04): `<scheme>:<value>`
/// where the scheme is one Terraform emits — `h1` (Go dirhash.Hash1:
/// STANDARD base64 of a 32-byte SHA-256, exactly 43 alphabet chars plus
/// one `=` pad) or `zh` (the zip package's SHA-256 as 64 LOWERCASE hex
/// chars, Go hex encoding) — and the value decodes to a digest of the
/// scheme's real canonical shape. Arbitrary junk like `not-a-checksum`,
/// `evil:x` or impossible lengths like `h1:a` must never become the
/// provider-set authority.
fn checksum_value(raw: &str, address: &str) -> CoreResult<String> {
    let Some((scheme, value)) = raw.split_once(':') else {
        return Err(err(format!(
            "lock file provider {address} has a malformed checksum pin {raw:?}"
        )));
    };
    let bytes = value.as_bytes();
    let valid = match scheme {
        "h1" => {
            bytes.len() == 44
                && bytes[43] == b'='
                && bytes[..43]
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/'))
        }
        "zh" => bytes.len() == 64 && bytes.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        _ => false,
    };
    if !valid {
        return Err(err(format!(
            "lock file provider {address} has an unsupported checksum pin {raw:?}"
        )));
    }
    Ok(raw.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    // Standard-base64 h1 digests (43 alphabet chars + '=') and a hex
    // sha256 zh digest — the exact shapes Go dirhash / Terraform emit.
    const H1_EMPTY: &str = "h1:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";
    const H1_OTHER: &str = "h1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const ZH_SHA256: &str = "zh:abababababababababababababababababababababababababababababababab";
    const LOCK: &str = "# generated by terraform init\n\nprovider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version     = \"2.23.0\"\n  constraints = \">= 1.9, < 2.0\"\n  hashes = [\n    \"h1:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=\",\n    \"h1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\",\n    \"zh:abababababababababababababababababababababababababababababababab\",\n  ]\n}\n\nprovider \"registry.terraform.io/kreuzwerker/docker\" {\n  version = \"3.0.2\"\n  hashes = [\n    \"h1:zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz=\",\n  ]\n}\n";

    #[test]
    fn parses_versions_and_checksum_commitments() {
        let providers = parse_lock_providers(LOCK).unwrap();
        assert_eq!(providers.len(), 2);
        let k8s = &providers[0];
        assert_eq!(k8s.source, "registry.terraform.io/hashicorp/kubernetes");
        assert_eq!(provider_source(&k8s.source), "hashicorp/kubernetes");
        assert_eq!(k8s.version, "2.23.0");
        // The commitment is order-insensitive: sorted hash list.
        assert_eq!(
            k8s.checksums_digest,
            format!("sha256:{}", {
                let mut hasher = Sha256::new();
                hasher.update(format!("{H1_EMPTY}\n{H1_OTHER}\n{ZH_SHA256}\n").as_bytes());
                hex::encode(hasher.finalize())
            })
        );
    }

    #[test]
    fn missing_version_or_unclosed_block_fails_closed() {
        let no_version = "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  hashes = [\n    \"h1:a\",\n  ]\n}\n";
        assert!(parse_lock_providers(no_version).is_err());
        let unclosed =
            "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"1.0.0\"\n";
        assert!(parse_lock_providers(unclosed).is_err());
        let stray = "provider registry.terraform.io/hashicorp/kubernetes\n";
        assert!(parse_lock_providers(stray).is_err());
    }

    #[test]
    fn checksum_less_provider_is_refused_as_authority() {
        // R5-03: version without hashes would authenticate as
        // SHA256(empty) — refuse it instead of projecting it.
        let no_hashes =
            "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"2.23.0\"\n}\n";
        assert!(parse_lock_providers(no_hashes).is_err());
        let empty_hashes = "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"2.23.0\"\n  hashes = [\n  ]\n}\n";
        assert!(parse_lock_providers(empty_hashes).is_err());
    }

    #[test]
    fn garbage_text_is_a_parse_error_not_an_empty_lock() {
        // R5-03: unrecognised top-level syntax must fail closed — only a
        // genuinely comment/blank-only file is a legal provider-free
        // lock.
        assert!(parse_lock_providers("this is not hcl at all\n{ random junk }\n").is_err());
        // Comments and blank lines ARE a legal empty lock.
        assert!(
            parse_lock_providers("# generated by terraform init\n\n# no providers\n")
                .unwrap()
                .is_empty()
        );
        // terraform settings blocks are skipped, contents not parsed.
        let with_tf = format!("terraform {{\n  required_version = \">= 1.9\"\n  garbage!!!\n}}\n\nprovider \"registry.terraform.io/hashicorp/kubernetes\" {{\n  version = \"2.23.0\"\n  hashes = [\"{H1_OTHER}\"]\n}}\n");
        assert_eq!(parse_lock_providers(&with_tf).unwrap().len(), 1);
    }

    #[test]
    fn bare_terraform_word_is_garbage_not_a_settings_block() {
        // R6-10: `terraform` alone (or with the brace on a later line)
        // is invalid syntax, not a skippable settings block — it must
        // never yield a legal-looking empty provider set.
        assert!(parse_lock_providers("terraform\n").is_err());
        assert!(parse_lock_providers("terraform\n{\n}\n").is_err());
        assert!(parse_lock_providers("terraform_config\n").is_err());
    }

    #[test]
    fn junk_checksums_are_rejected_not_hashed_into_a_commitment() {
        // R6-10: the pins are the provider-set authority, so only the
        // representations Terraform emits may enter a commitment.
        let junk = "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"2.23.0\"\n  hashes = [\"not-a-checksum\"]\n}\n";
        assert!(parse_lock_providers(junk).is_err());
        let evil_scheme = "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"2.23.0\"\n  hashes = [\"evil:x\"]\n}\n";
        assert!(parse_lock_providers(evil_scheme).is_err());
        let bad_chars = "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"2.23.0\"\n  hashes = [\"h1:!!!\"]\n}\n";
        assert!(parse_lock_providers(bad_chars).is_err());
        // The multi-line list form is validated too.
        let multiline = "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"2.23.0\"\n  hashes = [\n    \"not-a-checksum\",\n  ]\n}\n";
        assert!(parse_lock_providers(multiline).is_err());
        // R7-02: Terraform's Go dirhash.Hash1 uses STANDARD base64 — the
        // empty-digest sample Terraform itself documents must parse.
        let standard = format!("provider \"registry.terraform.io/hashicorp/kubernetes\" {{\n  version = \"2.23.0\"\n  hashes = [\n    \"{H1_EMPTY}\",\n    \"{ZH_SHA256}\",\n  ]\n}}\n");
        assert_eq!(parse_lock_providers(&standard).unwrap().len(), 1);
        // The URL-safe alphabet (- and _) is NOT what Terraform emits for
        // h1 pins; accepting it would have rejected real locks.
        let url_safe = "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"2.23.0\"\n  hashes = [\"h1:47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU=\"]\n}\n";
        assert!(parse_lock_providers(url_safe).is_err());
        // Impossible digest lengths are rejected, never hashed in.
        let short_h1 = "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"2.23.0\"\n  hashes = [\"h1:a\"]\n}\n";
        assert!(parse_lock_providers(short_h1).is_err());
        let short_zh = "provider \"registry.terraform.io/hashicorp/kubernetes\" {\n  version = \"2.23.0\"\n  hashes = [\"zh:a\"]\n}\n";
        assert!(parse_lock_providers(short_zh).is_err());
        // R8-04: zh is the zip's SHA-256 as 64 LOWERCASE hex — a 128-hex
        // (or uppercase) value can never equal that digest and must not
        // pass as a validated authority.
        let sha512_zh = format!("provider \"registry.terraform.io/hashicorp/kubernetes\" {{\n  version = \"2.23.0\"\n  hashes = [\"zh:{}\"]\n}}\n", "0f".repeat(64));
        assert!(parse_lock_providers(&sha512_zh).is_err());
        let uppercase_zh = format!("provider \"registry.terraform.io/hashicorp/kubernetes\" {{\n  version = \"2.23.0\"\n  hashes = [\"zh:{}\"]\n}}\n", "AB".repeat(32));
        assert!(parse_lock_providers(&uppercase_zh).is_err());
    }
}
