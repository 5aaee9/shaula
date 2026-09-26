use super::*;
use shaula_core::worker::{wire::*, ControlCapability, MAX_CONTROL_BYTES};

struct Control {
    id: Uuid,
    capability: ControlCapability,
    calls: AtomicUsize,
}

#[async_trait]
impl WorkerControl for Control {
    async fn authenticate(&self, id: Uuid, capability: &ControlCapability) -> StateResult<()> {
        if id != self.id || !capability.matches(&self.capability.verifier()) {
            return Err(StateError::Unauthorized);
        }
        Ok(())
    }
    async fn call(
        &self,
        capability: &ControlCapability,
        request: ControlRequest,
    ) -> StateResult<ControlResponse> {
        self.authenticate(request.claim.generation_id, capability)
            .await?;
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(ControlResponse::Ack)
    }
    async fn material(
        &self,
        id: Uuid,
        capability: &ControlCapability,
        _: Uuid,
    ) -> StateResult<Vec<u8>> {
        self.authenticate(id, capability).await?;
        Ok(b"{}".to_vec())
    }
}

#[tokio::test]
async fn internal_control_and_state_capabilities_never_cross_and_auth_precedes_body() -> TestResult
{
    let backend = Backend::new();
    let control = Arc::new(Control {
        id: backend.access.generation_id,
        capability: ControlCapability::issue(),
        calls: AtomicUsize::new(0),
    });
    // Exercise the actual router merge used by the bound private listener.
    let server = StateServer::bind("127.0.0.1:0".parse()?, backend.clone())
        .await?
        .with_worker_control(control.clone());
    let app = server.router;
    let uri = format!("/internal/v1/generations/{}/control", control.id);
    for auth in [
        backend.basic(),
        format!("Bearer {}", backend.access.capability.expose()),
        "Bearer personal-token".into(),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&uri)
                    .header("Authorization", auth)
                    .body(Body::from(vec![b'x'; MAX_CONTROL_BYTES + 1]))?,
            )
            .await?;
        assert_eq!(response.status(), 401);
        assert_eq!(response.headers()["cache-control"], "private, no-store");
    }
    let bearer = format!("Bearer {}", control.capability.expose());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(backend.uri())
                .header("Authorization", &bearer)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), 401);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header("Authorization", &bearer)
                .body(Body::from(vec![b'x'; MAX_CONTROL_BYTES + 1]))?,
        )
        .await?;
    assert_eq!(response.status(), 413);
    let request = ControlRequest {
        claim: shaula_core::state_backend::StateClaim {
            generation_id: control.id,
            worker_epoch: 1,
            worker_attempt: Uuid::new_v4(),
        },
        request_id: Uuid::new_v4(),
        message: ControlMessage::Handshake { protocol: 1 },
    };
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&uri)
                .header("Authorization", &bearer)
                .body(Body::from(serde_json::to_vec(&request)?))?,
        )
        .await?;
    assert_eq!(response.status(), 200);
    let wrong = format!("/internal/v1/generations/{}/control", Uuid::new_v4());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(wrong)
                .header("Authorization", bearer)
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), 401);
    assert_eq!(control.calls.load(Ordering::Relaxed), 1);
    assert_eq!(backend.reads.load(Ordering::Relaxed), 0);
    Ok(())
}
