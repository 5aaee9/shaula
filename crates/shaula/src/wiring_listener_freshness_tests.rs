//! Idle listeners keep the recorded demand evidence current: an empty long
//! poll on the live session confirms the stored snapshot without rewriting it.

use super::*;
use shaula_core::diagnostics::{timestamp, Freshness, QuestionId, SubjectKind};
use shaula_core::registry::AdmissionSupportStore;

async fn scale_up_at(plane: &TestPlane, at: i64) -> shaula_core::diagnostics::DiagnosticQuestion {
    let observed = timestamp(at).expect("valid test time");
    // The diagnostic snapshot writer persists asynchronously.
    for _ in 0..200 {
        let report = plane
            .control_plane
            .diagnostic_read(
                SubjectKind::Fleet,
                FLEET,
                &crate::wiring::wiring_tests::fleet_actor(),
                at,
            )
            .await
            .unwrap()
            .unwrap();
        if let Some(question) = report
            .questions
            .into_iter()
            .find(|q| q.question == QuestionId::ScaleUp)
            .filter(|q| q.basis.observed_at.as_deref() == Some(observed.as_str()))
        {
            return question;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("scale-up decision observed at {observed} was never projected");
}

#[tokio::test]
async fn empty_long_polls_keep_idle_demand_evidence_fresh() {
    let (plane, mut wiring, listener, clock, _mock) = setup().await;
    ready(&plane, &mut wiring, &clock).await;
    listener.emit_message.store(true, Ordering::SeqCst);
    tick(&mut wiring, &clock, NOW + 5_000).await;
    assert_eq!(
        plane.control_plane.demand_with_time(FLEET).await.unwrap(),
        Some((2, Some(NOW + 5_000)))
    );

    // Two idle minutes later the broker only answers 202: well past any
    // evidence window measured from the last statistics message.
    let idle = NOW + 125_000;
    let polls = listener.polls.load(Ordering::SeqCst);
    tick(&mut wiring, &clock, idle).await;
    assert!(listener.polls.load(Ordering::SeqCst) > polls);
    assert_eq!(
        plane.control_plane.demand_with_time(FLEET).await.unwrap(),
        Some((2, Some(idle))),
        "an empty poll confirms the snapshot time without changing demand"
    );

    let next = idle + 1_000;
    tick(&mut wiring, &clock, next).await;
    let question = scale_up_at(&plane, next).await;
    assert_eq!(question.basis.freshness, Freshness::Fresh);
    assert!(
        !question
            .reasons
            .iter()
            .any(|r| r.code == "observation.stale"),
        "idle fleet reported stale demand: {:?}",
        question.reasons
    );
    wiring.tasks.shutdown().await;
}
