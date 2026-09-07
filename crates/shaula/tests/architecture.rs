#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Architecture gate: inspects `cargo metadata`/`cargo tree` to prove the
//! production binary is pure Rust with no Go bridge/FFI, no Kubernetes or
//! Docker client, and no forbidden crate-to-crate dependency edges
//! (spec 0007 §2, §6).

use std::process::Command;

fn cargo_metadata_json() -> serde_json::Value {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata must run");
    assert!(output.status.success(), "cargo metadata failed");
    serde_json::from_slice(&output.stdout).expect("cargo metadata JSON invalid")
}

fn workspace_package_names() -> Vec<String> {
    cargo_metadata_json()["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .map(|p| p["name"].as_str().expect("package name").to_string())
        .collect()
}

#[test]
fn workspace_contains_the_nine_spec_crates() {
    let mut names = workspace_package_names();
    names.sort();
    for expected in [
        "shaula",
        "shaula-core",
        "shaula-daemon",
        "shaula-http",
        "shaula-scaleset",
        "shaula-store",
        "shaula-store-migration",
        "shaula-template",
        "shaula-observability",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "missing crate {expected}"
        );
    }
}

fn cargo_tree(package: &str, edges: &str) -> String {
    let output = Command::new(env!("CARGO"))
        .args([
            "tree",
            "-p",
            package,
            "--edges",
            edges,
            "--charset",
            "ascii",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree must run");
    assert!(output.status.success(), "cargo tree failed for {package}");
    String::from_utf8(output.stdout).expect("tree output utf8")
}

fn assert_forbidden(package: &str, tree: String, forbidden: &[&str]) {
    for needle in forbidden {
        assert!(
            !tree.contains(needle),
            "{package} must not depend on {needle} (found in tree)"
        );
    }
}

#[test]
fn core_has_no_adapter_framework_dependencies() {
    let tree = cargo_tree("shaula-core", "normal,build");
    assert_forbidden(
        "shaula-core",
        tree,
        &["axum", "sea-orm", "reqwest", "clap", "kube", "docker"],
    );
}

#[test]
fn daemon_has_no_adapter_dependencies() {
    let tree = cargo_tree("shaula-daemon", "normal,build");
    assert_forbidden(
        "shaula-daemon",
        tree,
        // The daemon sees platform access ONLY through core ports: the
        // concrete template adapter is wired by the composition root
        // (the shaula binary), never by the daemon itself.
        &[
            "axum",
            "sea-orm",
            "reqwest",
            "clap",
            "kube ",
            "docker ",
            "shaula-template",
            "shaula-scaleset",
        ],
    );
}

#[test]
fn http_never_touches_the_store() {
    let tree = cargo_tree("shaula-http", "normal,build");
    // ADR-0013 permits outbound HTTP only for the adapter's OIDC provider.
    assert_forbidden(
        "shaula-http",
        tree,
        &[
            "shaula-store",
            "sea-orm",
            "sqlx",
            "shaula-scaleset",
            "shaula-template",
        ],
    );
}

#[test]
fn scaleset_and_template_do_not_touch_seaorm() {
    let tree = cargo_tree("shaula-scaleset", "normal,build");
    assert_forbidden("shaula-scaleset", tree, &["sea-orm", "sqlx"]);
    let tree = cargo_tree("shaula-template", "normal,build");
    assert_forbidden(
        "shaula-template",
        tree,
        &["sea-orm", "sqlx", "kube", "docker"],
    );
}

#[test]
fn production_binary_is_rust_only() {
    let tree = cargo_tree("shaula", "normal,build");
    // No Go bridge or FFI runtime.
    for needle in [
        "go ",
        "cgo",
        "kube-client",
        "k8s-openapi",
        "docker-api",
        "bollard",
    ] {
        assert!(
            !tree.contains(needle),
            "shaula binary must not link {needle}"
        );
    }
}

#[test]
fn workspace_forbids_unsafe_code() {
    // REAL assertion (review R5, hardened R6/C18): the workspace root
    // must declare `rust.unsafe_code = "forbid"` and EVERY member crate
    // must inherit the workspace lints via `[lints] workspace = true` —
    // a member that drops the inheritance would silently escape the
    // forbid. Parsed STRUCTURALLY with `toml` (not string matching), so
    // `unsafe_code = "warn" # forbid` comments or whitespace variants
    // cannot fool the gate.
    const CRATES: &[&str] = &[
        "shaula",
        "shaula-core",
        "shaula-daemon",
        "shaula-http",
        "shaula-scaleset",
        "shaula-store",
        "shaula-store-migration",
        "shaula-template",
        "shaula-observability",
    ];
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/<name> layout");
    let root_toml: toml::Value = toml::from_str(
        &std::fs::read_to_string(workspace_root.join("Cargo.toml"))
            .expect("workspace Cargo.toml readable"),
    )
    .expect("workspace Cargo.toml valid TOML");
    let unsafe_code = root_toml
        .get("workspace")
        .and_then(|w| w.get("lints"))
        .and_then(|l| l.get("rust"))
        .and_then(|r| r.get("unsafe_code"))
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(
        unsafe_code, "forbid",
        "workspace lints.rust.unsafe_code must be exactly \"forbid\""
    );
    for name in CRATES {
        let member: toml::Value = toml::from_str(
            &std::fs::read_to_string(workspace_root.join("crates").join(name).join("Cargo.toml"))
                .unwrap_or_else(|e| panic!("{name} Cargo.toml readable: {e}")),
        )
        .unwrap_or_else(|e| panic!("{name} Cargo.toml valid TOML: {e}"));
        let inherits = member
            .get("lints")
            .and_then(|l| l.get("workspace"))
            .and_then(|w| w.as_bool())
            .unwrap_or(false);
        assert!(
            inherits,
            "{name} must inherit the workspace lints ([lints] workspace = true)"
        );
    }
}
