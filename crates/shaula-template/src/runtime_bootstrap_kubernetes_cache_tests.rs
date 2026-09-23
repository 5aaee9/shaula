use super::*;

#[test]
fn kubectl_keeps_discovery_cache_outside_frozen_template_material(
) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    let private = tempfile::tempdir()?;
    let mut request = super::super::tests::request();
    request.workspace_path = workspace.path().into();
    request
        .input
        .bindings
        .insert("kubeconfig".into(), json!("/etc/shaula/kubeconfig"));
    let prefix = kubectl_prefix(&request, private.path()).map_err(|_| "prefix rejected")?;
    let cache = prefix
        .windows(2)
        .find(|args| args[0] == "--cache-dir")
        .ok_or("missing explicit cache directory")?;
    let path = Path::new(&cache[1]);
    assert!(path.is_absolute() && path.starts_with(private.path()));
    assert!(!path.starts_with(workspace.path()));
    assert!(prefix
        .windows(2)
        .any(|args| args == ["--kubeconfig", "/etc/shaula/kubeconfig"]));
    assert!(kubectl_prefix(&request, workspace.path()).is_err());
    assert!(kubectl_prefix(&request, Path::new("relative")).is_err());
    Ok(())
}
