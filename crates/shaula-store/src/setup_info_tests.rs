use super::*;
type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn fixture() -> Result<(tempfile::TempDir, Store, String), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let store = Store::open(&temporary.path().join("test.db")).await?;
    store.migrate().await?;
    store.connection().execute_unprepared("INSERT INTO fleets(key,incarnation,desired_revision,observed_revision,mutation_fence,deletion_marker,phase,tombstone,created_at,updated_at) VALUES('fleet','incarnation',1,1,1,0,'Active',0,1,1)").await?;
    let generation = uuid::Uuid::new_v4().to_string();
    store.connection().execute(Statement::from_sql_and_values(DbBackend::Sqlite,
        "INSERT INTO runner_generations(id,fleet_key,runner_name,generation_name,fleet_revision,template_profile_key,template_revision,template_artifact_digest,attestation_id,inputs_digest,state,workspace_path,created_at,updated_at) VALUES(?,'fleet','runner','generation',1,'template',1,'artifact','activation','inputs','Creating','workspace',1,1)", [generation.clone().into()])).await?;
    Ok((temporary, store, generation))
}

fn config() -> SetupInfoConfig {
    SetupInfoConfig {
        listen: std::net::SocketAddr::from(([127, 0, 0, 1], 9091)),
        advertised_origin: "https://logs.test".into(),
        capability_ttl_seconds: 3600,
        wait_seconds: 60,
        requests_per_second: 2,
        burst: 4,
        max_concurrent_requests: 64,
    }
}

#[tokio::test]
async fn capability_survives_registry_restart_without_rotation_and_expires() -> TestResult {
    let (_temporary, store, generation) = fixture().await?;
    let registry = SetupInfoCapabilityRegistry::new(store.clone(), config())?;
    let real_millis = chrono::Utc::now().timestamp_millis();
    assert!(registry.issue(&generation, real_millis).await.is_err());
    let descriptor = registry.issue(&generation, 100).await?;
    let SetupInfoDescriptor::Enabled {
        capability,
        expires_at,
        ..
    } = descriptor
    else {
        return Err("descriptor disabled".into());
    };
    assert!(registry.authorize(&generation, &capability, 101).await?);
    assert!(registry.issue(&generation, 101).await.is_err());
    let restarted = SetupInfoCapabilityRegistry::new(store.clone(), config())?;
    assert!(restarted.authorize(&generation, &capability, 101).await?);
    assert!(
        !restarted
            .authorize(&generation, &capability, expires_at)
            .await?
    );
    assert!(
        !restarted
            .authorize(&uuid::Uuid::new_v4().to_string(), &capability, 101)
            .await?
    );
    assert!(
        !restarted
            .authorize(&generation, &"b".repeat(64), 101)
            .await?
    );
    let row = store
        .connection()
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT verifier FROM setup_info_capabilities",
        ))
        .await?
        .ok_or("missing verifier")?;
    let verifier: Vec<u8> = row.try_get("", "verifier")?;
    assert_eq!(verifier.len(), 32);
    assert_ne!(verifier, capability.as_bytes());
    store
        .connection()
        .execute(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "UPDATE runner_generations SET state='Destroyed' WHERE id=?",
            [generation.clone().into()],
        ))
        .await?;
    assert!(!restarted.authorize(&generation, &capability, 101).await?);
    Ok(())
}

#[tokio::test]
async fn lifecycle_milliseconds_convert_to_one_hour_unix_second_expiry() -> TestResult {
    let (_temporary, store, generation) = fixture().await?;
    let registry = SetupInfoCapabilityRegistry::new(store, config())?;
    let lifecycle_now = chrono::Utc::now().timestamp_millis();
    let seconds = lifecycle_now.div_euclid(1000);
    let SetupInfoDescriptor::Enabled {
        capability,
        expires_at,
        ..
    } = registry.issue(&generation, seconds).await?
    else {
        return Err("descriptor disabled".into());
    };
    assert_eq!(expires_at, seconds + 3600);
    assert!(
        registry
            .authorize(&generation, &capability, seconds + 3599)
            .await?
    );
    assert!(
        !registry
            .authorize(&generation, &capability, seconds + 3600)
            .await?
    );
    Ok(())
}

#[test]
fn setup_info_config_requires_origin_and_bounded_listener_limits() {
    let good = config();
    assert!(good.validate().is_ok());
    for origin in [
        "http://logs.test",
        "https://user:pass@logs.test",
        "https://logs.test/path",
        "https://logs.test?secret=x",
    ] {
        let mut invalid = good.clone();
        invalid.advertised_origin = origin.into();
        assert!(invalid.validate().is_err());
    }
    let mut invalid = good.clone();
    invalid.listen.set_ip(std::net::IpAddr::from([0, 0, 0, 0]));
    assert!(invalid.validate().is_err());
    let mut invalid = good;
    invalid.capability_ttl_seconds = 86401;
    assert!(invalid.validate().is_err());
}
