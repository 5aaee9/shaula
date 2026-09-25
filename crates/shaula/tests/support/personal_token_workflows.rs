//! CLI resource writes exercise the production router, service and SQLite.
use super::*;
use serde_json::{json, Value};

async fn cli(
    fixture: &Fixture,
    credential: &Secret,
    args: Vec<String>,
) -> Result<(i32, Value), Box<dyn std::error::Error>> {
    let origin = fixture.origin.clone();
    let credential = credential.clone();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_shaula"))
            .args([
                "--server",
                &origin,
                "--allow-loopback-http",
                "--output",
                "json",
            ])
            .args(args)
            .env("SHAULA_ACCESS_TOKEN", credential.expose())
            .env_remove("SHAULA_CONTEXT")
            .output()
    })
    .await??;
    let body = serde_json::from_slice(&output.stdout)?;
    Ok((output.status.code().ok_or("exit code")?, body))
}

#[tokio::test]
async fn github_and_forgejo_publication_rotation_versions_and_write_only_scope() -> TestResult {
    let fixture = Fixture::new().await?;
    let TokenIssue::Issued { secret, .. } = fixture
        .primary("ops")?
        .access_tokens()
        .issue(
            &request(vec![
                "auth.read".into(),
                "auth.write".into(),
                "auth.retire".into(),
            ]),
            "workflow".into(),
        )
        .await?
    else {
        return Err("issued".into());
    };
    let dir = tempfile::tempdir()?;
    for (key, body) in [
        (
            "github",
            json!({"kind":"github_app","schema_version":2,"app_id":"42","private_key":"private-fixture","target_policy":[{"kind":"organization","owner":"example"}]}),
        ),
        (
            "forgejo",
            json!({"kind":"forgejo_token","instance_url":"https://forgejo.example","token":"private-fixture","scope":{"kind":"organization","name":"example"}}),
        ),
    ] {
        let file = dir.path().join(format!("{key}.json"));
        std::fs::write(&file, serde_json::to_vec(&body)?)?;
        let (code, created) = cli(
            &fixture,
            &secret,
            vec![
                "auth-profiles".into(),
                "create".into(),
                key.into(),
                "--file".into(),
                file.to_string_lossy().into(),
            ],
        )
        .await?;
        assert_eq!(code, 0, "{key}: {created}");
        assert_eq!(created["receipt"]["outcome"], "accepted");
        let change = created["receipt"]["change"]["id"]
            .as_str()
            .ok_or("change")?;
        let (code, change) = cli(
            &fixture,
            &secret,
            vec![
                "changes".into(),
                "get".into(),
                change.into(),
                "--kind".into(),
                "profile".into(),
            ],
        )
        .await?;
        assert_eq!(code, 0, "{change}");
        assert_eq!(change["data"]["state"], "Pending");
        let (code, read) = cli(
            &fixture,
            &secret,
            vec!["auth-profiles".into(), "get".into(), key.into()],
        )
        .await?;
        assert_eq!(code, 0, "{read}");
        assert!(!read.to_string().contains("private-fixture"));
        let version = read["metadata"]["version"]
            .as_str()
            .ok_or("version")?
            .to_owned();
        let mut replacement = body;
        replacement[if key == "github" {
            "private_key"
        } else {
            "token"
        }] = json!("replacement-fixture");
        std::fs::write(&file, serde_json::to_vec(&replacement)?)?;
        let args = vec![
            "auth-profiles".into(),
            "rotate".into(),
            key.into(),
            "--file".into(),
            file.to_string_lossy().into(),
            "--if-match".into(),
            version,
            "--yes".into(),
        ];
        let (code, rotated) = cli(&fixture, &secret, args.clone()).await?;
        assert_eq!(code, 0, "{rotated}");
        assert_eq!(rotated["receipt"]["outcome"], "accepted");
        let (code, conflict) = cli(&fixture, &secret, args).await?;
        assert_eq!(code, 5, "{conflict}");
    }
    let TokenIssue::Issued { secret, .. } = fixture
        .primary("ops")?
        .access_tokens()
        .issue(&request(vec!["auth.write".into()]), "write-only".into())
        .await?
    else {
        return Err("issued".into());
    };
    let file = dir.path().join("github.json");
    let (code, created) = cli(
        &fixture,
        &secret,
        vec![
            "auth-profiles".into(),
            "create".into(),
            "write-only".into(),
            "--file".into(),
            file.to_string_lossy().into(),
        ],
    )
    .await?;
    assert_eq!(code, 0, "{created}");
    let (code, denied) = cli(
        &fixture,
        &secret,
        vec!["auth-profiles".into(), "get".into(), "write-only".into()],
    )
    .await?;
    assert_eq!(code, 4, "{denied}");
    Ok(())
}
