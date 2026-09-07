use super::*;
use crate::secret::SecretString;

fn bytes() -> Vec<u8> {
    br#"{"version":4,"lineage":"lineage","serial":0,"resources":[],"outputs":{}}"#.to_vec()
}

#[test]
fn state_roundtrips_raw_bytes_and_redacts_debug() -> StateResult<()> {
    let raw = bytes();
    let document = StateDocument::parse(raw.clone())?;
    assert_eq!(document.bytes(), raw);
    assert_eq!(document.serial(), 0);
    assert!(document.managed_empty());
    assert_eq!(format!("{document:?}"), "StateDocument(REDACTED)");
    Ok(())
}

#[test]
fn state_rejects_malformed_headers_and_ambiguous_empty_state() -> StateResult<()> {
    for raw in [
        r#"{"version":4,"version":4,"lineage":"a","serial":0,"resources":[],"outputs":{}}"#,
        r#"{"version":4,"lineage":"a","serial":0,"serial":1,"resources":[],"outputs":{}}"#,
        r#"{"version":4,"lineage":"a","serial":18446744073709551615,"resources":[],"outputs":{}}"#,
        r#"{"version":4,"lineage":"a","serial":-1,"resources":[],"outputs":{}}"#,
        r#"{"version":3,"lineage":"a","serial":0,"resources":[],"outputs":{}}"#,
        r#"{"version":4,"lineage":"","serial":0,"resources":[],"outputs":{}}"#,
        r#"{"version":4,"lineage":"a","serial":0,"outputs":{}}"#,
        r#"{"version":4,"lineage":"a","serial":0,"resources":null,"outputs":{}}"#,
        r#"{"format_version":"1.0","values":{"root_module":{}}}"#,
    ] {
        assert!(matches!(
            StateDocument::parse(raw.as_bytes().to_vec()),
            Err(StateError::Invalid)
        ));
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes()).map_err(|_| StateError::Invalid)?;
    value["resources"] = serde_json::json!([{
        "mode": "managed", "type": "test", "name": "runner", "provider": "test",
        "instances": [{}]
    }]);
    assert!(!StateDocument::parse(value.to_string().into_bytes())?.managed_empty());
    value["resources"][0]["instances"] = serde_json::json!([null]);
    assert!(StateDocument::parse(value.to_string().into_bytes()).is_err());
    value["resources"][0]["instances"] = serde_json::json!([]);
    value["resources"][0]["mode"] = serde_json::json!("unknown");
    assert!(StateDocument::parse(value.to_string().into_bytes()).is_err());
    assert!(matches!(
        StateDocument::parse(vec![0; MAX_STATE_BYTES + 1]),
        Err(StateError::TooLarge)
    ));
    Ok(())
}

#[test]
fn state_capabilities_are_separate_random_and_redacted() -> StateResult<()> {
    let first = StateCapability::issue();
    let second = StateCapability::issue();
    assert_ne!(first.expose(), second.expose());
    assert!(first.matches(&first.verifier()));
    assert!(!first.matches(&second.verifier()));
    let parsed = StateCapability::parse(SecretString::new(first.expose()))?;
    assert!(parsed.matches(&first.verifier()));
    assert!(!format!("{first:?}").contains(first.expose()));
    for other in ["ordinary-oidc-jwt", "control-token", "ss1_short"] {
        assert!(matches!(
            StateCapability::parse(SecretString::new(other)),
            Err(StateError::Unauthorized)
        ));
    }
    Ok(())
}

#[test]
fn lock_metadata_is_bounded_strict_and_never_debug_printed() -> StateResult<()> {
    let info = LockInfo::parse(br#"{"ID":"lock-secret","Who":"user-secret","Path":"/secret"}"#)?;
    assert_eq!(info.id().expose(), "lock-secret");
    assert_eq!(info, LockInfo::parse(&info.to_bytes()?)?);
    assert!(!format!("{info:?}").contains("secret"));
    assert!(!format!("{:?}", StateError::Locked(Box::new(info))).contains("secret"));
    for invalid in [
        r#"{}"#,
        r#"{"ID":""}"#,
        r#"{"ID":"a","ID":"b"}"#,
        r#"{"ID":"a","owner":true}"#,
    ] {
        assert!(LockInfo::parse(invalid.as_bytes()).is_err());
    }
    assert!(matches!(
        LockInfo::parse(&vec![0; MAX_LOCK_BYTES + 1]),
        Err(StateError::TooLarge)
    ));
    Ok(())
}
