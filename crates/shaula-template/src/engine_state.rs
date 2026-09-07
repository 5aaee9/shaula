//! Raw-state v4 parsing (statefile/version4.go contract), split
//! from engine_flow to stay within the 400-line limit (AGENTS.md).

use shaula_core::error::{CoreResult, ReasonCode};

use super::err;

/// Parses the RAW Terraform state (format version 4) into the trusted
/// state identity. Unknown or corrupt structure is REJECTED, never read
/// as empty (fail closed, spec 0004 §5).
pub(crate) fn parse_raw_state_v4(
    state: &serde_json::Value,
) -> CoreResult<crate::workspace::StateSnapshot> {
    let obj = state.as_object().ok_or_else(|| {
        err(
            ReasonCode::TemplateExecutionFailed,
            "state is not an object",
        )
    })?;
    let version = obj.get("version").and_then(|v| v.as_u64()).ok_or_else(|| {
        err(
            ReasonCode::TemplateExecutionFailed,
            "state format version missing",
        )
    })?;
    if version != 4 {
        return Err(err(
            ReasonCode::TemplateExecutionFailed,
            format!("unsupported state format version {version}"),
        ));
    }
    let serial = obj
        .get("serial")
        .and_then(|s| s.as_u64())
        .ok_or_else(|| err(ReasonCode::TemplateExecutionFailed, "state serial missing"))?;
    let lineage = obj
        .get("lineage")
        .and_then(|l| l.as_str())
        .filter(|l| !l.is_empty())
        .ok_or_else(|| {
            err(
                ReasonCode::TemplateExecutionFailed,
                "state lineage missing or empty; identity cannot be proven",
            )
        })?
        .to_string();

    // Managed INSTANCE addresses only — data sources persist in state
    // but are never deletion targets (spec 0004 §5.193: MANAGED-state
    // coverage). In the raw format ALL resources live at the TOP LEVEL,
    // each carrying its `module` path and per-instance `index_key`
    // (count/for_each). The plan JSON keeps the same instance-level
    // addresses, so exact-state coverage compares like for like.
    let mut managed = Vec::new();
    if let Some(resources) = obj.get("resources") {
        collect_resource_array(resources, &mut managed)?;
    }
    managed.sort();
    managed.dedup();
    Ok(crate::workspace::StateSnapshot {
        managed,
        serial,
        lineage,
    })
}

/// The `module` field of a raw-state resource: `module.a`, a nested
/// `module.a.module.b` chain, or a chain whose multi-instance modules
/// carry instance suffixes (`module.runners["east.us"]`, `module.m[0]`).
/// A dot INSIDE a quoted for_each key is data, never a separator — the
/// suffix is consumed quote- and escape-aware (R6-05). Anything else is
/// a corrupt state. Returns the equivalent address prefix (with
/// trailing dot), empty for root.
fn validated_module_prefix(module: Option<&str>) -> CoreResult<String> {
    let Some(module) = module else {
        return Ok(String::new());
    };
    if module.is_empty() {
        return Err(err(
            ReasonCode::TemplateExecutionFailed,
            "state module path is empty",
        ));
    }
    let mut prefix = String::new();
    let mut rest = module;
    loop {
        let Some(after) = rest.strip_prefix("module.") else {
            return Err(err(
                ReasonCode::TemplateExecutionFailed,
                format!("state module path {module:?} is malformed"),
            ));
        };
        // The module NAME ends at the first `.` or `[` (a bare name
        // cannot contain either); the split is done on the RAW text so
        // dots inside a LATER quoted suffix never truncate the name.
        let name_end = after.find(['.', '[']).unwrap_or(after.len());
        let name = &after[..name_end];
        if name.is_empty() {
            return Err(err(
                ReasonCode::TemplateExecutionFailed,
                format!("state module path {module:?} has an empty module name"),
            ));
        }
        prefix.push_str("module.");
        prefix.push_str(name);
        let mut remainder = &after[name_end..];
        // One optional instance suffix per chain element, consumed to
        // its closing `]` (quotes and backslash escapes inside are
        // data, so a dotted key like "east.us" survives intact).
        if let Some(suffix) = remainder.strip_prefix('[') {
            remainder = consume_instance_suffix(suffix, &mut prefix)?;
        }
        // The returned prefix always ends with a trailing dot, ready to
        // carry the resource address (`module.runners.<type>.<name>`).
        prefix.push('.');
        match remainder.strip_prefix('.') {
            None => {
                if remainder.is_empty() {
                    break;
                }
                return Err(err(
                    ReasonCode::TemplateExecutionFailed,
                    format!("state module path {module:?} is malformed"),
                ));
            }
            Some(next) => rest = next,
        }
    }
    Ok(prefix)
}

/// Consumes one `["key"]` / `[0]` instance suffix (the opening `[` has
/// already been stripped) from `rest`, appending it verbatim to
/// `prefix`; returns what remains after the closing `]`.
fn consume_instance_suffix<'a>(rest: &'a str, prefix: &mut String) -> CoreResult<&'a str> {
    prefix.push('[');
    let mut in_quote = false;
    let mut escaped = false;
    for (i, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            prefix.push(ch);
            continue;
        }
        match ch {
            '\\' if in_quote => {
                escaped = true;
                prefix.push(ch);
            }
            '"' => {
                in_quote = !in_quote;
                prefix.push(ch);
            }
            ']' if !in_quote => {
                prefix.push(']');
                return Ok(&rest[i + 1..]);
            }
            _ => prefix.push(ch),
        }
    }
    Err(err(
        ReasonCode::TemplateExecutionFailed,
        "state module path has an unterminated instance suffix",
    ))
}

/// The instance-level address suffix for a raw-state instance:
/// count/for_each `index_key` renders as `[0]` / `["key"]`; an absent
/// key is the single unindexed instance.
fn instance_address(
    base: &str,
    instance: &serde_json::Map<String, serde_json::Value>,
) -> CoreResult<String> {
    match instance.get("index_key") {
        None => Ok(base.to_string()),
        Some(serde_json::Value::Number(n)) => Ok(format!("{base}[{n}]")),
        Some(serde_json::Value::String(key)) => Ok(format!("{base}[{key:?}]")),
        Some(_) => Err(err(
            ReasonCode::TemplateExecutionFailed,
            format!("state instance index_key malformed at {base}"),
        )),
    }
}

/// The TOP-LEVEL raw-state `resources` array. Each managed instance
/// contributes its FULL address (`module.x.type.name["key"]`); a
/// resource with zero instances contributes nothing — it holds no live
/// object and must not fabricate a deletion target.
fn collect_resource_array(resources: &serde_json::Value, into: &mut Vec<String>) -> CoreResult<()> {
    let resources = resources.as_array().ok_or_else(|| {
        err(
            ReasonCode::TemplateExecutionFailed,
            "state resources malformed",
        )
    })?;
    for resource in resources {
        let resource = resource.as_object().ok_or_else(|| {
            err(
                ReasonCode::TemplateExecutionFailed,
                "state resource malformed",
            )
        })?;
        let mode = resource
            .get("mode")
            .and_then(|m| m.as_str())
            .ok_or_else(|| {
                err(
                    ReasonCode::TemplateExecutionFailed,
                    "state resource mode missing",
                )
            })?;
        if mode != "managed" && mode != "data" {
            return Err(err(
                ReasonCode::TemplateExecutionFailed,
                format!("unknown state resource mode {mode:?}"),
            ));
        }
        let resource_type = resource
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        let name = resource
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default();
        if resource_type.is_empty() || name.is_empty() {
            return Err(err(
                ReasonCode::TemplateExecutionFailed,
                "state resource type/name missing",
            ));
        }
        let module_prefix =
            validated_module_prefix(resource.get("module").and_then(|m| m.as_str()))?;
        let base = format!("{module_prefix}{resource_type}.{name}");
        // `instances` is mandatory in the raw format; a missing field is
        // corrupt state, not an empty one.
        let instances = resource
            .get("instances")
            .and_then(|i| i.as_array())
            .ok_or_else(|| {
                err(
                    ReasonCode::TemplateExecutionFailed,
                    format!("state instances malformed at {base}"),
                )
            })?;
        if mode == "managed" {
            for instance in instances {
                let instance = instance.as_object().ok_or_else(|| {
                    err(
                        ReasonCode::TemplateExecutionFailed,
                        format!("state instance malformed at {base}"),
                    )
                })?;
                into.push(instance_address(&base, instance)?);
            }
        }
    }
    Ok(())
}
