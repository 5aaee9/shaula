//! THE canonical content-addressed artifact directory layout, shared by
//! every reader of the store (publisher, validator, supervisor, template
//! runtime): `<root>/<hex[0..2]>/<hex>`. One authority — no adapter may
//! re-derive the layout.

use std::path::{Path, PathBuf};

/// Resolves the directory of one published artifact under `root`.
///
/// Strict digest validation before any path operation: a mutable local
/// directory must never impersonate a published artifact (spec 0005
/// section 4, ARD-0009). Non-hex input (including non-ASCII that would
/// panic on byte slicing) yields `None` instead of a path.
pub fn artifact_dir(root: &Path, digest: &str) -> Option<PathBuf> {
    let hex_part = digest.strip_prefix("sha256:")?;
    if hex_part.len() != 64
        || !hex_part
            .bytes()
            .all(|b| matches!(b, b'a'..=b'f' | b'0'..=b'9'))
    {
        return None;
    }
    Some(root.join(&hex_part[..2]).join(hex_part))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const HEX64: &str = "a3f1c0d2e3b4a5968778695a4b3c2d1e0f9a8b7c6d5e4f3a2b1c0d9e8f7a6b5c";

    #[test]
    fn valid_digest_shards_two_levels() {
        let dir = artifact_dir(Path::new("/artifacts"), &format!("sha256:{HEX64}")).unwrap();
        assert_eq!(dir, Path::new("/artifacts").join("a3").join(HEX64));
    }

    #[test]
    fn malformed_digest_yields_no_path() {
        assert!(artifact_dir(Path::new("/a"), "sha256:short").is_none());
        assert!(artifact_dir(Path::new("/a"), "no-prefix-and-short").is_none());
        let upper = format!("sha256:{}", HEX64.to_uppercase());
        assert!(artifact_dir(Path::new("/a"), &upper).is_none());
        // Non-ASCII must never be byte-sliced into a path.
        assert!(artifact_dir(
            Path::new("/a"),
            "sha256:énonascii-nonascii-nonascii-nonascii"
        )
        .is_none());
    }
}
