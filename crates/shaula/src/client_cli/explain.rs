use super::*;
use shaula_client::types::diagnostics::{DiagnosticReportV1, SubjectKind};

pub(super) async fn run(
    client: &Client,
    kind: &str,
    key: &str,
    watch: bool,
    timeout: u64,
    output: Option<Output>,
) -> Result<Outcome, Error> {
    let subject = match kind {
        "fleets" => SubjectKind::Fleet,
        "generations" => SubjectKind::Generation,
        "jobs" => SubjectKind::Job,
        _ => {
            return Err(Error::Invalid(
                "explain supports fleets, generations and jobs",
            ))
        }
    };
    let end = tokio::time::Instant::now()
        .checked_add(std::time::Duration::from_secs(timeout))
        .ok_or(Error::Invalid("watch timeout is too large"))?;
    loop {
        let report = client.diagnostics(subject, key).await?;
        if !watch {
            return Ok(Outcome::data(report.raw));
        }
        if matches!(output, Some(Output::Table))
            || (output.is_none() && std::io::stdout().is_terminal())
        {
            table(&report.data);
        } else {
            event(
                "diagnostics",
                serde_json::to_value(&report.data).map_err(|_| Error::Protocol)?,
                report.data.truncated,
            );
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => { let mut out = Outcome::data(report.raw); out.code = 130; return Ok(out); },
            _ = tokio::time::sleep_until(end) => return Ok(Outcome::data(report.raw)),
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
        }
    }
}

pub(super) fn table(report: &DiagnosticReportV1) {
    println!("QUESTION\tOUTCOME\tCOVERAGE\tFRESHNESS\tOBSERVED AT\tREASON");
    for question in &report.questions {
        let primary = question
            .primary_reason_id
            .as_ref()
            .and_then(|id| question.reasons.iter().find(|r| &r.id == id));
        println!(
            "{:?}\t{:?}\t{:?}\t{:?}\t{}\t{}",
            question.question,
            question.outcome,
            question.coverage,
            question.basis.freshness,
            question.basis.observed_at.as_deref().unwrap_or("unknown"),
            primary
                .map(|r| r.code.escape_default().to_string())
                .unwrap_or_else(|| "unknown".into())
        );
        for reason in &question.reasons {
            let known = shaula_core::diagnostics::Code::ALL
                .iter()
                .find(|c| c.as_str() == reason.code);
            println!(
                "  {}: {}",
                reason.code.escape_default(),
                known
                    .map(|c| c.definition().3)
                    .unwrap_or("The current client does not recognize this reason")
            );
        }
    }
}
