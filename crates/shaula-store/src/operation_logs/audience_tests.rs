use super::*;

#[test]
fn operator_diagnostics_cannot_smuggle_progress_markers_to_workflows() {
    for line in [
        "Error: private endpoint failed while runner: Creating...",
        "Unexpected response: [REDACTED] private infrastructure details",
        "Error: echoed [Shaula: operator-only diagnostic]",
        "docker_container.runner: Creation complete after 1s private-details",
        "Apply complete! Resources: private-details",
    ] {
        assert_eq!(
            runner_line(line),
            "[Shaula: diagnostic withheld from workflow audience]"
        );
    }
    assert_eq!(
        runner_line("docker_container.runner: Creation complete after 1s"),
        "docker_container.runner: Creation complete after 1s"
    );
    assert_eq!(
        runner_line("Apply complete! Resources: 1 added, 0 changed, 0 destroyed."),
        "Apply complete! Resources: 1 added, 0 changed, 0 destroyed."
    );
}

#[tokio::test]
async fn supported_historical_policy_remains_readable_and_unknown_is_withheld(
) -> Result<(), Box<dyn std::error::Error>> {
    let (temporary, store, generation) = super::super::tests::fixture().await?;
    let archive =
        OperationLogArchive::open(store, temporary.path().join("logs"), LogConfig::default())
            .await?;
    let id = archive
        .begin(BeginInvocation {
            generation_id: generation,
            operation: "Create".into(),
            started_at: 1,
        })
        .await?;
    archive
        .append(AppendLog {
            invocation_id: id.clone(),
            command_ordinal: 0,
            phase: "plan".into(),
            stream: "stderr".into(),
            sequence: 0,
            text: "Error: already sanitized\n".into(),
            observed_at: 1,
            withheld: false,
        })
        .await?;
    for policy in [
        "shaula.operation-text/v1",
        "shaula.operation-text/v2",
        "unknown-policy",
    ] {
        let mut record = archive.load(&id).await?;
        record.policy_version = policy.into();
        archive.save(&record).await?;
        let page = archive.read_page(&id, LogQuery::default()).await?;
        if policy == "unknown-policy" {
            assert_eq!(page.capture_status, "withheld");
            assert!(page.entries.is_empty());
        } else {
            assert_eq!(page.entries.len(), 1, "{policy}");
            assert_eq!(page.entries[0].text, "Error: already sanitized\n");
        }
    }
    Ok(())
}
