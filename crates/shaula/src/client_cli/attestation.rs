//! Immutable attestation writes retain the attempt even when the ACK is lost.
use super::*;

pub(super) async fn execute(
    client: &Client,
    attempt: MutationAttempt,
    key: &str,
    revision: i64,
    attestation: &str,
) -> Outcome {
    let retry_key = attempt.idempotency_key().to_owned();
    let (mut out, outcome) = match client
        .execute::<shaula_client::types::Document>(&attempt)
        .await
    {
        Ok(result) => (
            Outcome {
                version: result.version.map(|v| v.as_str().to_owned()),
                ..Outcome::data(result.data)
            },
            "completed",
        ),
        Err(error) => {
            let uncertain = matches!(
                error,
                Error::Transport
                    | Error::Protocol
                    | Error::Http {
                        status: 500..=599,
                        ..
                    }
            );
            let out = Outcome::error(error);
            if !uncertain {
                return out;
            }
            (out, "uncertain")
        }
    };
    out.receipt = json!({
        "outcome": outcome, "resource_kind": "template-attestation",
        "resource_key": key, "revision": revision, "attestation_key": attestation,
        "idempotency_key": retry_key
    });
    out
}
