use super::*;

fn output(sanitizer: &mut Sanitizer, text: &str) -> String {
    sanitizer
        .feed(text.as_bytes())
        .into_iter()
        .map(|record| record.0)
        .collect()
}

#[test]
fn operator_diagnostics_retain_provider_error_title_body_and_source_location() {
    let mut sanitizer = Sanitizer::new(std::sync::Arc::new(SensitiveValues::default()));
    let diagnostic = concat!(
        "Error: Failed to load plugin schemas\n\n",
        "Error while loading schemas for plugin components: Failed to obtain provider\n",
        "schema: Could not load the schema for provider registry.terraform.io/indexyz/proxmox.\n",
        "\n  with proxmox_qemu_vm.runner,\n",
        "  on main.tf line 42, in resource \"proxmox_qemu_vm\" \"runner\":\n",
        "The provider does not support the resource type requested by this configuration.\n",
    );
    assert_eq!(output(&mut sanitizer, diagnostic), diagnostic);
}

#[test]
fn provider_installation_context_survives_outside_an_error_paragraph() {
    let mut sanitizer = Sanitizer::new(std::sync::Arc::new(SensitiveValues::default()));
    let text = concat!(
        "- Finding indexyz/proxmox versions matching \"0.4.0\"...\n",
        "- Installing indexyz/proxmox v0.4.0...\n",
        "- Installed indexyz/proxmox v0.4.0 (self-signed, key ID 0123456789ABCDEF)\n",
    );
    assert_eq!(output(&mut sanitizer, text), text);
}

#[test]
fn useful_diagnostic_context_survives_while_secret_fragments_are_redacted() {
    let values = SensitiveValues::from_input(
        &serde_json::json!({"bindings":{"token":"provision-secret"}}),
        &[],
    );
    let mut sanitizer = Sanitizer::new(std::sync::Arc::new(values));
    assert!(sanitizer
        .feed(b"Error: Provider authentication failed for provision-")
        .is_empty());
    let records = sanitizer
        .feed(b"secret\nThe remote endpoint rejected the supplied credential (HTTP 401).\n");
    let text = records
        .into_iter()
        .map(|record| record.0)
        .collect::<String>();
    assert!(text.contains("Error: Provider authentication failed for [REDACTED]"));
    assert!(text.contains("remote endpoint rejected"));
    assert!(!text.contains("provision-secret"));
}

#[test]
fn diagnostic_context_never_turns_structured_dumps_or_pem_into_log_text() {
    let mut sanitizer = Sanitizer::new(std::sync::Arc::new(SensitiveValues::default()));
    let text = output(
        &mut sanitizer,
        concat!(
            "Error: Remote API failed\n",
            "password = arbitrary-new-secret\n",
            "{\n  \"opaque\": \"unknown-credential-value\"\n}\n",
            "-----BEGIN PRIVATE KEY-----\n",
            "unknownPrivateKeyMaterialAcrossRecords\n",
            "-----END PRIVATE KEY-----\n",
            "Authorization: Bearer arbitrary-bearer-value\n",
            "::set-output name=value::injected\n",
        ),
    );
    for secret in [
        "arbitrary-new-secret",
        "unknown-credential-value",
        "unknownPrivateKeyMaterial",
        "arbitrary-bearer-value",
        "injected",
    ] {
        assert!(!text.contains(secret), "released {secret}: {text}");
    }
}
