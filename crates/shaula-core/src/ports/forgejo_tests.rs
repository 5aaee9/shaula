use super::*;
use crate::secret::SecretString;

#[test]
fn bootstrap_material_redacts_and_validates() -> Result<(), &'static str> {
    let material = ForgejoBootstrapMaterial::new(
        "https://forgejo.example.test",
        "runner-uuid",
        SecretString::new("one-shot-token"),
        vec!["linux:docker://alpine".into()],
    )?;
    let debug = format!("{material:?}");
    assert!(debug.contains("REDACTED"));
    assert!(!debug.contains("one-shot-token"));
    assert!(ForgejoBootstrapMaterial::new(
        "https://forgejo.example.test/?token=leak",
        "runner-uuid",
        SecretString::new("token"),
        Vec::new(),
    )
    .is_err());
    Ok(())
}
