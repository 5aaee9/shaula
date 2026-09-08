//! Static, bounded discovery of the Terraform input declaration. No expression is evaluated.

use std::collections::BTreeSet;
use std::path::Path;

use hcl::Expression;
use shaula_core::error::{CoreError, CoreResult, ReasonCode};
use shaula_core::registry::TemplateVariables;

#[path = "variables_constraints.rs"]
mod constraints;
#[path = "variables_guard.rs"]
mod guard;
#[path = "variables_numbers.rs"]
mod numbers;
#[path = "variables_schema.rs"]
mod schema;
#[path = "variables_type.rs"]
mod types;

const MAX_BYTES: usize = 1024 * 1024;
const MAX_FIELDS: usize = 256;
const MAX_DEPTH: usize = 16;

fn invalid(reason: &'static str) -> CoreError {
    CoreError::new(ReasonCode::TemplateInvalid, reason)
}

fn read_text(path: &Path, budget: &mut usize) -> CoreResult<String> {
    let file =
        std::fs::File::open(path).map_err(|_| invalid("template variable source is unreadable"))?;
    let bytes = crate::artifact::read_bounded(file, MAX_BYTES as u64)?;
    *budget = budget.saturating_add(bytes.len());
    if *budget > MAX_BYTES {
        return Err(invalid("template variable source exceeds the 1 MiB limit"));
    }
    String::from_utf8(bytes).map_err(|_| invalid("template variable source is not UTF-8"))
}

/// Reads only root module `.tf` declarations and the two published schemas.
/// Legacy `type = any` is reported explicitly without invalidating its artifact.
pub fn discover_variables(dir: &Path, digest: &str) -> CoreResult<TemplateVariables> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|_| invalid("template variable source directory is unreadable"))?
    {
        let entry = entry.map_err(|_| invalid("template variable source is unreadable"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".tf.json") || name == "override.tf" || name.ends_with("_override.tf") {
            return Err(invalid(
                "Terraform JSON and override files are unsupported for variable discovery",
            ));
        }
        if !name.ends_with(".tf") {
            continue;
        }
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            return Err(invalid(
                "template variable source must contain regular files",
            ));
        }
        paths.push(entry.path());
        if paths.len() > 128 {
            return Err(invalid("template variable source exceeds the file limit"));
        }
    }
    paths.sort_unstable();
    let mut declaration = None;
    let mut bytes = 0;
    for path in paths {
        let text = read_text(&path, &mut bytes)?;
        guard::check(&text)?;
        let parsed: hcl::edit::structure::Body = text
            .parse()
            .map_err(|_| invalid("Terraform source contains invalid HCL"))?;
        // The preserving AST retains spelling and comments, but duplicate object keys
        // overwrite their earlier entry. Refuse any lost source before using that tree.
        if parsed.to_string() != text {
            return Err(invalid(
                "Terraform source contains ambiguous or lossy object declarations",
            ));
        }
        for block in parsed
            .into_blocks()
            .filter(|b| b.ident.as_str() == "variable")
        {
            if declaration.is_some()
                || block.labels.len() != 1
                || block.labels[0].as_str() != "shaula"
            {
                return Err(invalid("template must declare exactly one shaula variable"));
            }
            declaration = Some(block.body);
        }
    }
    let declaration =
        declaration.ok_or_else(|| invalid("template must declare exactly one shaula variable"))?;
    let mut seen = BTreeSet::new();
    let mut sensitive = false;
    let mut type_expr = None;
    for attribute in declaration.into_attributes() {
        if !seen.insert(attribute.key.as_str().to_owned()) {
            return Err(invalid("shaula variable contains duplicate attributes"));
        }
        match attribute.key.as_str() {
            "sensitive" => sensitive = attribute.value.as_bool() == Some(true),
            "type" => {
                numbers::verify(&attribute.value)?;
                type_expr = Some(Expression::from(attribute.value));
            }
            "default" => {
                return Err(invalid(
                    "the system shaula variable cannot declare a default",
                ))
            }
            _ => {}
        }
    }
    if !sensitive {
        return Err(invalid("the shaula variable must declare sensitive = true"));
    }
    let expression = type_expr.ok_or_else(|| invalid("shaula variable type is absent"))?;
    let mut result = TemplateVariables {
        artifact_digest: digest.to_owned(),
        available: false,
        reason: None,
        bindings: Vec::new(),
        parameters: Vec::new(),
    };
    if matches!(&expression, Expression::Variable(value) if value.as_str() == "any") {
        result.reason = Some("This template declares shaula as any; typed bindings and parameters are required to discover variables.".into());
        return Ok(result);
    }
    let envelope = types::envelope(&expression)?;
    for (name, output) in [
        ("bindings", &mut result.bindings),
        ("parameters", &mut result.parameters),
    ] {
        let declaration = envelope
            .get(name)
            .ok_or_else(|| invalid("shaula variable is missing a system envelope member"))?;
        let json = read_text(
            &dir.join("schemas").join(format!("{name}.schema.json")),
            &mut bytes,
        )?;
        *output = schema::project(declaration, &json, name == "bindings")?;
    }
    result.available = true;
    let encoded =
        serde_json::to_vec(&result).map_err(|_| invalid("template variables cannot be encoded"))?;
    if encoded.len() > MAX_BYTES {
        return Err(invalid(
            "template variable projection exceeds the 1 MiB limit",
        ));
    }
    Ok(result)
}

#[cfg(test)]
#[path = "variables_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "variables_constraints_tests.rs"]
mod constraints_tests;

#[cfg(test)]
#[path = "variables_integrity_tests.rs"]
mod integrity_tests;

#[cfg(test)]
#[path = "variables_guard_tests.rs"]
mod guard_tests;
