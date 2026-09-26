use super::*;
use shaula_client::{
    types::Document, MutationOptions, Resource, ResourceVersion, WritePrecondition,
};
use std::path::Path;

pub(super) fn body(file: Option<&Path>) -> Result<Document, Error> {
    let raw = std::fs::read_to_string(file.ok_or(Error::Invalid(
        "--file is required; supply an API request JSON document",
    ))?)
    .map_err(|_| Error::Invalid("cannot read request file"))?;
    Document::parse(raw).map_err(|_| Error::Invalid("invalid JSON request"))
}
pub(super) fn options(args: &WriteArgs, create: bool) -> Result<MutationOptions, Error> {
    let precondition = if create {
        if args.if_match.is_some() {
            return Err(Error::Invalid("create does not accept --if-match"));
        }
        WritePrecondition::CreateOnly
    } else {
        WritePrecondition::Match(ResourceVersion::parse(args.if_match.as_deref().ok_or(
            Error::Invalid("--if-match must be the version reviewed with this request"),
        )?)?)
    };
    Ok(MutationOptions {
        precondition,
        idempotency_key: args
            .idempotency_key
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
    })
}
pub(super) async fn get(
    client: &Client,
    kind: &str,
    key: &str,
) -> Result<Resource<Document>, Error> {
    match kind {
        "fleets" => client.fleets().get(key).await,
        "templates" => client.templates().get(key).await,
        "pools" => client.pools().get(key).await,
        "auth-profiles" => client.auth_profiles().get(key).await,
        _ => Err(Error::Invalid("unknown resource")),
    }
}
pub(super) fn change_kind(kind: &str) -> ChangeKind {
    match kind {
        "fleets" => ChangeKind::Fleet,
        "pools" => ChangeKind::TemplatePool,
        _ => ChangeKind::Profile,
    }
}
pub(super) async fn run(
    client: &Client,
    kind: &str,
    action: ResourceAction,
) -> Result<Outcome, Error> {
    let read = match action {
        ResourceAction::Explain {
            key,
            watch,
            timeout,
        } => return super::explain::run(client, kind, &key, watch, timeout, None).await,
        ResourceAction::List => match kind {
            "fleets" => client.fleets().list().await?,
            "templates" => client.templates().list().await?,
            "pools" => client.pools().list().await?,
            _ => client.auth_profiles().list().await?,
        },
        ResourceAction::Get { key } => get(client, kind, &key).await?,
        ResourceAction::Status {
            key,
            watch,
            timeout,
        } => {
            if !["fleets", "auth-profiles"].contains(&kind) {
                return Err(Error::Invalid(
                    "status is available for fleets and auth-profiles",
                ));
            }
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout);
            loop {
                let result = if kind == "fleets" {
                    client.fleets().status(&key).await?
                } else {
                    client.auth_profiles().status(&key).await?
                };
                if !watch {
                    break result;
                }
                event(
                    "snapshot",
                    result.data.decode().map_err(|_| Error::Protocol)?,
                    false,
                );
                if tokio::time::Instant::now() >= deadline {
                    return Ok(Outcome::data(Value::Null));
                }
                tokio::select! {_=tokio::time::sleep(std::time::Duration::from_secs(2))=>{},_=tokio::signal::ctrl_c()=>{return Ok(Outcome{code:130,partial:true,..Outcome::data(Value::Null)});}}
            }
        }
        ResourceAction::Create(args) => return write(client, kind, "create", args).await,
        ResourceAction::Publish(args) => return write(client, kind, "publish", args).await,
        ResourceAction::Update(args) => return write(client, kind, "update", args).await,
        ResourceAction::Rotate(args) => return write(client, kind, "rotate", args).await,
        ResourceAction::PolicyUpdate(args) => {
            return write(client, kind, "policy-update", args).await
        }
        ResourceAction::Retire(args) => return write(client, kind, "retire", args).await,
        ResourceAction::Edit {
            key,
            wait,
            idempotency_key,
        } => return super::edit::run(client, kind, &key, wait, idempotency_key).await,
        ResourceAction::Sources { .. } if kind == "templates" => {
            client.templates().sources().await?
        }
        ResourceAction::Variables { digest } if kind == "templates" => {
            client.templates().variables(&digest).await?
        }
        ResourceAction::InputContract { key, revision } if kind == "templates" => {
            client.templates().input_contract(&key, revision).await?
        }
        ResourceAction::Revisions {
            action: RevisionAction::Get { key, revision },
        } if kind == "templates" => client.templates().revision(&key, revision).await?,
        ResourceAction::Revisions {
            action: RevisionAction::Get { key, revision },
        } if kind == "auth-profiles" => client.auth_profiles().revision(&key, revision).await?,
        ResourceAction::Impact { key } if kind == "auth-profiles" => {
            client.auth_profiles().impact(&key).await?
        }
        ResourceAction::InstallationLink { key } if kind == "auth-profiles" => {
            client.auth_profiles().installation_link(&key).await?
        }
        ResourceAction::Revisions {
            action: RevisionAction::Publish(args),
        } if kind == "templates" => return write(client, kind, "publish", args).await,
        ResourceAction::Upload { digest, file }
        | ResourceAction::Artifacts {
            action: ArtifactAction::Upload { digest, file },
        } if kind == "templates" => {
            client
                .templates()
                .upload(
                    &digest,
                    std::fs::read(file).map_err(|_| Error::Invalid("cannot read archive"))?,
                )
                .await?
        }
        ResourceAction::Attestations {
            action,
            key,
            revision,
            attestation,
            file,
            if_match,
            idempotency_key,
        } if kind == "templates" => {
            if action == "get" {
                client
                    .templates()
                    .attestation(&key, revision, &attestation)
                    .await?
            } else {
                let options = MutationOptions {
                    precondition: match if_match {
                        Some(v) => WritePrecondition::Match(ResourceVersion::parse(&v)?),
                        None => WritePrecondition::CreateOnly,
                    },
                    idempotency_key: idempotency_key
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                };
                let attempt = client
                    .templates()
                    .attest(
                        &key,
                        revision,
                        &attestation,
                        &body(file.as_deref())?,
                        options,
                    )
                    .await?;
                client.execute(&attempt).await?
            }
        }
        _ => return Err(Error::Invalid("operation is unavailable for this resource")),
    };
    Ok(Outcome {
        version: read.version.map(|v| v.as_str().to_owned()),
        ..Outcome::data(read.data)
    })
}
pub(super) async fn write(
    client: &Client,
    kind: &str,
    action: &str,
    args: WriteArgs,
) -> Result<Outcome, Error> {
    if ["rotate", "policy-update"].contains(&action) && kind != "auth-profiles" {
        return Err(Error::Invalid("operation requires auth-profiles"));
    }
    if kind == "auth-profiles" && ["rotate", "policy-update"].contains(&action) {
        let impact = client.auth_profiles().impact(&args.key).await?;
        eprintln!(
            "Current affected fleets: {}",
            serde_json::to_string(impact.data.raw()).map_err(|_| Error::Protocol)?
        );
    }
    if ["retire", "rotate", "policy-update"].contains(&action) {
        confirm(args.yes, &format!("{action} {kind}/{}", args.key))?;
    }
    if args.wait {
        let read_scope = match kind {
            "fleets" => "fleet.read",
            "pools" => "template.read",
            "templates" => "template.read",
            _ => "auth.read",
        };
        if !client
            .session()
            .await?
            .data
            .scopes
            .iter()
            .any(|s| s == read_scope)
        {
            return Err(Error::Invalid("--wait requires the resource read scope"));
        }
    }
    let create = action == "create" || action == "publish" && args.if_match.is_none();
    let opts = options(&args, create)?;
    let attempt = if action == "retire" {
        match kind {
            "fleets" => client.fleets().retire(&args.key, opts).await?,
            "templates" => client.templates().retire(&args.key, opts).await?,
            "pools" => client.pools().retire(&args.key, opts).await?,
            _ => client.auth_profiles().retire(&args.key, opts).await?,
        }
    } else {
        let body = body(args.file.as_deref())?;
        match kind {
            "fleets" => client.fleets().put(&args.key, &body, opts).await?,
            "templates" if action == "update" => {
                client.templates().update(&args.key, &body, opts).await?
            }
            "templates" => client.templates().put(&args.key, &body, opts).await?,
            "pools" => client.pools().put(&args.key, &body, opts).await?,
            _ if action == "policy-update" => {
                client
                    .auth_profiles()
                    .policy_update(&args.key, &body, opts)
                    .await?
            }
            _ => client.auth_profiles().put(&args.key, &body, opts).await?,
        }
    };
    Ok(mutation(
        client,
        attempt,
        change_kind(kind),
        &args.key,
        args.wait,
        args.timeout,
    )
    .await)
}
