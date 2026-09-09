//! Scripted target and effect endpoints used by composition tests.
#![allow(clippy::unwrap_used)]

use axum::{
    routing::{get, post},
    Json, Router,
};
use std::sync::{
    atomic::{AtomicI64, AtomicUsize, Ordering},
    Arc, Mutex,
};

pub(crate) struct Routes {
    pub router: Router,
    pub repo_id: Arc<AtomicI64>,
    pub repo_owner_id: Arc<AtomicI64>,
    pub scale_set_creates: Arc<AtomicUsize>,
    pub installation_reads: Arc<Mutex<Vec<i64>>>,
}

pub(crate) fn routes(
    listener: Option<Arc<crate::auth_worker_mock::listener::ListenerMock>>,
) -> Routes {
    let repo_id = Arc::new(AtomicI64::new(700));
    let repo_owner_id = Arc::new(AtomicI64::new(220));
    let scale_set_creates = Arc::new(AtomicUsize::new(0));
    let installation_reads: Arc<Mutex<Vec<i64>>> = Arc::default();
    let repo = repo_id.clone();
    let owner = repo_owner_id.clone();
    let creates = scale_set_creates.clone();
    let mut router = Router::new()
        .route(
            "/repos/5aaee9/proj",
            get(move || {
                let repo = repo.clone();
                let owner = owner.clone();
                async move {
                    Json(serde_json::json!({
                        "id": repo.load(Ordering::SeqCst), "name":"proj",
                        "owner":{"id":owner.load(Ordering::SeqCst),"login":"5aaee9"}
                    }))
                }
            }),
        )
        .route(
            "/repos/5aaee9/proj/installation",
            get(|| async {
                Json(crate::auth_worker_v2::tests::installation(
                    22, 4863460, "5aaee9", "selected",
                ))
            }),
        )
        .route(
            "/repos/5aaee9/proj/actions/runners/registration-token",
            post(|| async {
                (
                    axum::http::StatusCode::CREATED,
                    Json(serde_json::json!({"token":"registration"})),
                )
            }),
        )
        .route(
            "/orgs/Indexyz",
            get(|| async { Json(serde_json::json!({"id":110,"login":"Indexyz"})) }),
        )
        .route(
            "/actions/_apis/runtime/runnerscalesets",
            get(move || {
                let listener = listener.clone();
                async move {
                    Json(match listener {
                        Some(listener) => {
                            serde_json::json!({"count":1,"value":[listener.scale_set()]})
                        }
                        None => serde_json::json!({"count":0,"value":[]}),
                    })
                }
            })
            .post(move || {
                let creates = creates.clone();
                async move {
                    creates.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::BAD_REQUEST
                }
            }),
        );
    for installation in [11, 22, 23] {
        let reads = installation_reads.clone();
        router = router.route(
            &format!("/app/installations/{installation}"),
            get(move || {
                let reads = reads.clone();
                async move {
                    reads.lock().unwrap().push(installation);
                    let mut body = crate::auth_worker_v2::tests::installation(
                        installation,
                        4863460,
                        if installation == 11 {
                            "Indexyz"
                        } else {
                            "5aaee9"
                        },
                        "all",
                    );
                    if installation == 23 {
                        body["account"]["id"] = serde_json::json!(220);
                    }
                    Json(body)
                }
            }),
        );
    }
    router=router.route("/app/installations/23/access_tokens",post(|| async {
        (axum::http::StatusCode::CREATED,Json(serde_json::json!({"token":"inst-23-token","expires_at":"2027-01-01T00:00:00Z"})))
    }));
    Routes {
        router,
        repo_id,
        repo_owner_id,
        scale_set_creates,
        installation_reads,
    }
}
