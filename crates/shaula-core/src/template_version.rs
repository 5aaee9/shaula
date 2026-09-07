//! Engine-version constraint grammar: ONE fallible parser shared by the
//! structural manifest gate and the actual version check, split from
//! `template.rs` to keep every file within the 400-line limit
//! (AGENTS.md, C16/R5-06).

use crate::error::{CoreError, CoreResult, ReasonCode};

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::new(ReasonCode::TemplateInvalid, msg)
}

type Segments = (u64, u64, u64);

/// Parses `major[.minor[.patch]]` into zero-padded segments. Anything
/// non-numeric, empty or over-long is an Err — never a panic, even for
/// multi-byte UTF-8 input (R5-06).
fn parse_version(raw: &str) -> CoreResult<Segments> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(err("version bound is empty"));
    }
    let mut segments: Vec<u64> = Vec::new();
    for part in trimmed.split('.') {
        let segment = part.trim();
        let parsed = segment
            .parse::<u64>()
            .map_err(|_| err(format!("malformed version segment {segment:?}")))?;
        segments.push(parsed);
    }
    if segments.len() > 3 {
        return Err(err(format!(
            "version {trimmed:?} must be major.minor.patch"
        )));
    }
    while segments.len() < 3 {
        segments.push(0);
    }
    Ok((segments[0], segments[1], segments[2]))
}

/// One comma-separated constraint element: `>= bound` or `< bound`
/// followed by a parseable bound. Prefix matching is CHAR-safe
/// (`strip_prefix`), so no byte index can land inside a multi-byte
/// character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConstraintElement {
    AtLeast(Segments),
    Below(Segments),
}

/// The supported required-version grammar: comma-separated `>= a.b.c`
/// and `< a.b.c` constraints (what the bundled profiles declare). Any
/// other form fails closed at manifest validation.
fn parse_constraint(constraint: &str) -> CoreResult<Vec<ConstraintElement>> {
    let mut elements = Vec::new();
    for raw in constraint.split(',') {
        let raw = raw.trim();
        let (op, bound) = if let Some(bound) = raw.strip_prefix(">=") {
            (">=", bound)
        } else if let Some(bound) = raw.strip_prefix('<') {
            ("<", bound)
        } else {
            return Err(err(format!("unsupported version constraint {raw:?}")));
        };
        if bound.trim().is_empty() {
            return Err(err(format!(
                "version constraint {raw:?} has an empty bound"
            )));
        }
        let bound = parse_version(bound)?;
        elements.push(match op {
            ">=" => ConstraintElement::AtLeast(bound),
            _ => ConstraintElement::Below(bound),
        });
    }
    if elements.is_empty() {
        return Err(err("version constraint is empty"));
    }
    Ok(elements)
}

/// Does `actual` satisfy the constraint? Malformed input on either side
/// is a validation Err, never a panic.
pub fn version_satisfies(actual: &str, constraint: &str) -> CoreResult<()> {
    let actual = parse_version(actual)?;
    for element in parse_constraint(constraint)? {
        match element {
            ConstraintElement::AtLeast(bound) if actual < bound => {
                return Err(err(format!(
                    "version {actual:?} is below the required lower bound {bound:?}"
                )));
            }
            ConstraintElement::Below(bound) if actual >= bound => {
                return Err(err(format!(
                    "version {actual:?} is at or above the exclusive upper bound {bound:?}"
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Manifest validation rejects required_version forms outside the
/// supported grammar, so an unsatisfiable/unknown constraint can never
/// reach admission. Structural check via the SAME parser as
/// `version_satisfies` (one grammar owner, C16) — intentionally not a
/// probe: probe errors cannot distinguish "unsupported grammar" from
/// "bound violated" and would fail open.
pub(crate) fn required_version_supported(constraint: &str) -> bool {
    parse_constraint(constraint).is_ok()
}

#[cfg(test)]
mod version_tests {
    use super::{parse_version, required_version_supported, version_satisfies};

    #[test]
    fn bundled_range_grammar_is_supported() {
        assert!(version_satisfies("1.9.8", ">= 1.9, < 2.0").is_ok());
        assert!(version_satisfies("2.0.0", ">= 1.9, < 2.0").is_err());
        assert!(version_satisfies("1.8.9", ">= 1.9, < 2.0").is_err());
    }

    #[test]
    fn unsupported_grammar_fails_closed() {
        assert!(version_satisfies("1.9.8", "~> 1.9").is_err());
        assert!(version_satisfies("1.9.8", ">= 1.9").is_ok());
    }

    #[test]
    fn manifest_gate_rejects_unsupported_constraints_structurally() {
        // Structural check, not probe-based: these must be rejected even
        // though probe versions would make the failures ambiguous.
        assert!(required_version_supported(">= 1.9, < 2.0"));
        assert!(required_version_supported(">= 1.9"));
        assert!(required_version_supported("< 2.0"));
        assert!(!required_version_supported("~> 1.9"));
        assert!(!required_version_supported("= 1.9.0"));
        assert!(!required_version_supported("*"));
        assert!(!required_version_supported(""));
        assert!(!required_version_supported(">= 1.9, ~> 2.0"));
        assert!(!required_version_supported(">= x.y"));
        assert!(!required_version_supported(">="));
        assert!(!required_version_supported(">= 1.2.3.4"));
    }

    #[test]
    fn multi_byte_input_is_a_validation_error_not_a_panic() {
        // R5-06: `中` is three bytes; a byte-index split at 2 would land
        // inside the character and panic. Every entry point must return
        // an Err instead.
        assert!(!required_version_supported("中"));
        assert!(!required_version_supported(">= 中"));
        assert!(!required_version_supported(">= 1.9, 中"));
        assert!(version_satisfies("1.9.8", "中").is_err());
        assert!(version_satisfies("中", ">= 1.9").is_err());
        assert!(version_satisfies("1.9.8", "< 2.0, >= 1.9, <b").is_err());
        // Partial ASCII prefixes of multi-byte text never panic either.
        assert!(parse_version("1.9.八").is_err());
    }
}
