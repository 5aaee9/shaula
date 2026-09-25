use shaula_api_types::*;
type Result = std::result::Result<(), Box<dyn std::error::Error>>;
#[test]
fn writes_preserve_number_tokens_and_omission_and_reject_unknown_envelopes() -> Result {
    let raw = r#"{"capacity":{"min_runners":0,"max_runners":1},"template_inputs":{"large":18446744073709551617,"precise":1.2345678901234567890123456789}}"#;
    let typed: FleetPut = serde_json::from_str(raw)?;
    let roundtrip = serde_json::to_string(&typed)?;
    assert!(roundtrip.contains("18446744073709551617"));
    assert!(roundtrip.contains("1.2345678901234567890123456789"));
    assert!(serde_json::from_str::<FleetPut>(r#"{"capacity":{},"unknown":true}"#).is_err());
    let missing: TemplateUpdate =
        serde_json::from_str(r#"{"artifact_digest":"sha256:a","engine_ref":"engine"}"#)?;
    assert!(!serde_json::to_string(&missing)?.contains("bindings"));
    assert!(serde_json::from_str::<TemplateUpdate>(
        r#"{"artifact_digest":"a","engine_ref":"b","bindings":null}"#
    )
    .is_err());
    let explicit: TemplateUpdate = serde_json::from_str(
        r#"{"artifact_digest":"a","engine_ref":"b","bindings":{"keep":null,"empty":{}}}"#,
    )?;
    assert_eq!(
        explicit.bindings.ok_or("bindings")?.raw(),
        r#"{"keep":null,"empty":{}}"#
    );
    Ok(())
}
#[test]
fn wire_receipts_are_distinct_and_unknown_states_are_preserved() -> Result {
    let normal: NormalMutation = serde_json::from_str(
        r#"{"changeId":"","state":"NoOp","revision":3,"noOp":true,"future":1}"#,
    )?;
    assert!(normal.no_op);
    let change: Change = serde_json::from_str(
        r#"{"id":"c","resourceKind":"fleet","resourceKey":"k","revision":1,"kind":"Update","state":"FutureState"}"#,
    )?;
    assert_eq!(change.state, "FutureState");
    let secret = Secret::new("do-not-expose".into());
    assert!(!format!("{secret:?}").contains("do-not-expose"));
    Ok(())
}
