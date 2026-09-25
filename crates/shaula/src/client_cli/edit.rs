use super::*;
use shaula_client::types::Document;
use std::io::Write;

pub(super) async fn run(
    client: &Client,
    kind: &str,
    key: &str,
    wait: bool,
    idempotency_key: Option<String>,
) -> Result<Outcome, Error> {
    let resource = resources::get(client, kind, key).await?;
    let version = resource
        .version
        .ok_or(Error::Invalid("resource has no strong write version"))?;
    let fields: std::collections::BTreeMap<String, Box<serde_json::value::RawValue>> =
        resource.data.decode().map_err(|_| Error::Protocol)?;
    let initial = match kind {
        "fleets" | "pools" => fields.get("spec").ok_or(Error::Protocol)?.get().to_owned(),
        // Updates inherit omitted bindings; never feed redacted secrets into PUT.
        "templates" => {
            let revision: i64 =
                serde_json::from_str(fields.get("desiredRevision").ok_or(Error::Protocol)?.get())
                    .map_err(|_| Error::Protocol)?;
            let base: Value = client
                .templates()
                .revision(key, revision)
                .await?
                .data
                .decode()
                .map_err(|_| Error::Protocol)?;
            serde_json::to_string_pretty(&json!({"source_key":base["sourceKey"],"artifact_digest":base["artifactDigest"],"engine_ref":base["engineRef"]})).map_err(|_| Error::Protocol)?
        }
        _ => {
            return Err(Error::Invalid(
                "auth rotation requires an explicit --file; use impact before preparing it",
            ))
        }
    };
    let directory =
        tempfile::tempdir().map_err(|_| Error::Invalid("cannot create editor directory"))?;
    let private = directory.path().join("private");
    super::secure_file::private_directory(&private)?;
    let path = private.join("request.json");
    let mut file = super::secure_file::create(&path)?;
    file.write_all(initial.as_bytes())
        .map_err(|_| Error::Invalid("cannot prepare editor input"))?;
    drop(file);
    let editor = std::env::var_os("VISUAL")
        .or_else(|| std::env::var_os("EDITOR"))
        .ok_or(Error::Invalid(
            "set VISUAL or EDITOR to the editor executable path",
        ))?;
    // An executable and a path are distinct argv entries. No shell evaluation.
    let mut process = std::process::Command::new(editor);
    process.arg(&path).env_clear();
    for key in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "HOME",
        "USERPROFILE",
        "TERM",
    ] {
        if let Some(value) = std::env::var_os(key) {
            process.env(key, value);
        }
    }
    if !process
        .status()
        .map_err(|_| Error::Invalid("cannot start editor executable"))?
        .success()
    {
        return Err(Error::Invalid("editor failed; nothing was submitted"));
    }
    let edited = Document::parse(
        std::fs::read_to_string(&path).map_err(|_| Error::Invalid("cannot read edited request"))?,
    )
    .map_err(|_| Error::Invalid("edited JSON is invalid"))?;
    if edited.raw() == initial {
        return Ok(Outcome::data(json!({"unchanged":true})));
    }
    confirm(
        false,
        &format!("update {kind}/{key} at {}", version.as_str()),
    )?;
    resources::write(
        client,
        kind,
        "update",
        WriteArgs {
            key: key.into(),
            file: Some(path),
            if_match: Some(version.as_str().into()),
            idempotency_key,
            wait,
            timeout: 300,
            yes: true,
        },
    )
    .await
}
