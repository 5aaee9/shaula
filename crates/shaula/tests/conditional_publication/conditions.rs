use axum::http::StatusCode;
use tower::ServiceExt;

use super::{common, registry::*, support::*};
use shaula_core::registry::{AuthProfilePut, MutationError, ProfileRegistryPort};

#[tokio::test]
async fn missing_stale_and_create_only_conditions_preserve_the_existing_contract() -> TestResult {
    let fixture = Fixture::new().await?;
    for kind in KINDS {
        let missing = Conditions {
            create: false,
            expected: None,
            idem: None,
        };
        assert_eq!(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                missing.clone()
            )
            .await?,
            Err(MutationError::PreconditionRequired)
        );
        let base = success(
            kind.publish(
                &fixture.service,
                &actor(),
                &fixture.digest,
                0,
                Conditions::create(),
            )
            .await?,
        )?;
        let before = fixture.count("audit_records").await?;
        assert_eq!(
            kind.publish(&fixture.service, &actor(), &fixture.digest, 0, missing)
                .await?,
            Err(MutationError::PreconditionRequired)
        );
        let mut stale = Conditions::replace(&base, "stale")?;
        stale.expected.as_mut().ok_or("expected version")?.1 = 0;
        for conditions in [stale, Conditions::create()] {
            assert!(matches!(
                kind.publish(&fixture.service, &actor(), &fixture.digest, 0, conditions)
                    .await?,
                Err(MutationError::PreconditionFailed { .. })
            ));
        }
        assert_eq!(kind.revision(&fixture).await?, 1);
        assert_eq!(fixture.count("audit_records").await?, before);
    }
    Ok(())
}

#[tokio::test]
async fn unsupported_auth_formats_are_rejected_before_historical_lookup() -> TestResult {
    let fixture = Fixture::new().await?;
    for kind in [Kind::Github, Kind::Forgejo] {
        let mut create = Conditions::create();
        create.idem = Some("existing-key".into());
        success(
            kind.publish(&fixture.service, &actor(), &fixture.digest, 0, create)
                .await?,
        )?;
        let github = matches!(kind, Kind::Github);
        let payload = AuthProfilePut {
            kind: if github {
                shaula_core::auth::AuthKind::GithubApp
            } else {
                shaula_core::auth::AuthKind::ForgejoToken
            },
            schema_version: Some(if github { 1 } else { 2 }),
            app_id: github.then(|| "4863460".into()),
            secret: shaula_core::secret::SecretString::new("unused"),
            target_policy: None,
            forgejo_target: None,
        };
        let result = fixture
            .service
            .auth_put(
                &actor(),
                kind.key(),
                payload,
                true,
                None,
                Some("existing-key".into()),
            )
            .await?;
        assert!(
            matches!(result, Err(MutationError::Unprocessable { .. })),
            "not a replay or key conflict: {result:?}"
        );
        assert_eq!(kind.revision(&fixture).await?, 1);
    }
    Ok(())
}

#[tokio::test]
async fn mixed_conditions_are_rejected_before_admission_for_every_resource() -> TestResult {
    let (app, _, _) = common::build_app_with_scan().await;
    let template =
        common::TEMPLATE_PUT_BODY.replace("PLACEHOLDER", &format!("sha256:{}", "a".repeat(64)));
    for (uri, body) in [
        ("/api/v1/fleets/new", common::FLEET_BODY),
        (POOL_URI, POOL_BODY),
        ("/api/v1/template-profiles/new", template.as_str()),
        ("/api/v1/github-auth-profiles/github", common::AUTH_PUT_BODY),
        (
            "/api/v1/github-auth-profiles/forgejo",
            r#"{"kind":"forgejo_token","instance_url":"https://forgejo.example.test","scope":{"kind":"instance"},"token":"private-forgejo-token"}"#,
        ),
    ] {
        let mut request = common::put_with_idempotency(uri, "mixed", body.into());
        request
            .headers_mut()
            .insert("if-match", "\"inc:1\"".parse()?);
        let response = app.clone().oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await?;
        let problem: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(problem["code"], "ConflictingPreconditions");
        let response = app
            .clone()
            .oneshot(common::authorized("GET", uri, None))
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }
    Ok(())
}
