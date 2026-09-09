//! Historical rows are injected below the public boundary to model an existing DB.

use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub async fn database(engine: &std::path::Path) -> TestResult<DatabaseConnection> {
    let path = engine
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or("fixture engine has no root")?
        .join("data/test.db");
    Ok(Database::connect(format!(
        "sqlite://{}?mode=rw",
        path.to_string_lossy().replace('\\', "/")
    ))
    .await?)
}

pub async fn seed(engine: &std::path::Path, key: &str, kind: &str) -> TestResult {
    let db = database(engine).await?;
    db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
        "INSERT INTO github_auth_profiles(key,incarnation,desired_revision,active_revision,status,deletion_requested,created_at,updated_at) VALUES (?, ?, 1, 1, 'Active', 0, 1, 1)",
        [key.into(), format!("{key}-inc").into()])).await?;
    db.execute(Statement::from_sql_and_values(DatabaseBackend::Sqlite,
        "INSERT INTO github_auth_profile_revisions(profile_key,revision,kind,app_id,installation_id,pat_principal,allowlist_json,credential_bytes,state,created_at,schema_version,policy_json) VALUES (?,1,?,'4863460',34,'historical-principal',?,?,'Active',1,1,?)",
        [key.into(), kind.into(), r#"{"targets":[{"kind":"organization","owner":"example-org"}]}"#.into(), b"historical-inert-credential".to_vec().into(), r#"{"selectors":[{"kind":"organization","owner":"example-org"}]}"#.into()])).await?;
    db.close().await?;
    Ok(())
}
