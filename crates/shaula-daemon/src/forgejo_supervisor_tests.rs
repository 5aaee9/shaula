use super::*;
use shaula_core::ports::forgejo::ForgejoRunnerRef;

#[test]
fn backend_targets_are_not_runner_label_names() -> Result<(), shaula_core::error::CoreError> {
    let labels = normalized_labels(&[
        "linux:docker://alpine".to_string(),
        "x86_64:host".to_string(),
    ])?;
    assert_eq!(labels, ["linux", "x86_64"]);
    Ok(())
}

#[test]
fn empty_backend_target_is_rejected() {
    assert!(normalized_labels(&["linux:".to_string()]).is_err());
}

#[test]
fn ownership_requires_ephemeral_known_and_all_labels() {
    let labels = vec!["linux".to_string(), "x86_64".to_string()];
    let base = ForgejoRunnerRef {
        id: 1,
        uuid: "runner".into(),
        name: "fleet-1-generation".into(),
        status: "idle".into(),
        labels: labels.clone(),
        ephemeral: true,
        version: Some("13".into()),
    };
    assert!(is_owned_runner(&base, &labels));
    assert!(!is_owned_runner(
        &ForgejoRunnerRef {
            ephemeral: false,
            ..base.clone()
        },
        &labels
    ));
    assert!(!is_owned_runner(
        &ForgejoRunnerRef {
            status: "future".into(),
            ..base.clone()
        },
        &labels
    ));
    assert!(!is_owned_runner(
        &ForgejoRunnerRef {
            labels: vec!["linux".into()],
            ..base
        },
        &labels
    ));
}
