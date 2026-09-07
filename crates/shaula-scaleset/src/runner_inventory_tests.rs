use super::*;

#[test]
fn inconsistent_inventory_never_proves_absence() -> Result<(), Box<dyn std::error::Error>> {
    for value in [
        serde_json::json!({"count": 1, "value": []}),
        serde_json::json!({"count": -1, "value": []}),
        serde_json::json!({"count": 0, "value": [{"id": 1, "name": "r", "runnerScaleSetId": 2}]}),
        serde_json::json!({"count": 1, "value": [{"id": 1, "name": "r", "runnerScaleSetId": 0}]}),
    ] {
        let list: wire::RunnerReferenceList = serde_json::from_value(value)?;
        assert!(validate_runner_list(&list).is_err());
    }
    let list = serde_json::from_value(serde_json::json!({"count": 0, "value": []}))?;
    assert!(validate_runner_list(&list).is_ok());
    Ok(())
}
