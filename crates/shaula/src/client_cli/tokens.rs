use super::*;
use shaula_client::{
    types::{IssueToken, TokenIssue},
    ResourceVersion,
};
fn ttl(raw: &str) -> Result<u64, Error> {
    let (number, multiplier) = if let Some(n) = raw.strip_suffix('d') {
        (n, 86400)
    } else if let Some(n) = raw.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = raw.strip_suffix('s') {
        (n, 1)
    } else {
        (raw, 1)
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(multiplier))
        .filter(|n| (60..=31_536_000).contains(n))
        .ok_or(Error::Invalid(
            "TTL must be 60 seconds to 365 days; use 30d, 24h or seconds",
        ))
}
async fn issue(
    client: &Client,
    args: IssueArgs,
) -> Result<(Outcome, Option<shaula_client::Secret>), Error> {
    if args.secret_out.is_none() && !args.show_secret {
        return Err(Error::Invalid(
            "choose --secret-out <new-file> or explicitly --show-secret before issuance",
        ));
    }
    let body = IssueToken {
        name: args.name,
        scopes: args.scopes,
        expires_in_seconds: Some(ttl(&args.ttl)?),
    };
    let mut file = args
        .secret_out
        .as_ref()
        .map(|path| super::secure_file::create(path))
        .transpose()?;
    let key = args
        .idempotency_key
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let issued = match client.access_tokens().issue(&body, key.clone()).await {
        Ok(issued) => issued,
        Err(e) => {
            drop(file.take());
            if let Some(path) = &args.secret_out {
                let _ = std::fs::remove_file(path);
            }
            let uncertain = matches!(
                e,
                Error::Transport
                    | Error::Protocol
                    | Error::Http {
                        status: 500..=599,
                        ..
                    }
            );
            let mut out = Outcome::error(e);
            if uncertain {
                out.receipt = json!({"outcome":"uncertain","idempotency_key":key});
            }
            return Ok((out, None));
        }
    };
    let mut out = Outcome::data(issued.metadata());
    out.receipt = json!({"outcome":"completed","resource_kind":"access-token","resource_key":issued.metadata().id,"idempotency_key":key});
    match issued {
        TokenIssue::RecoveredWithoutSecret(_) => {
            drop(file.take());
            if let Some(path) = &args.secret_out {
                let _ = std::fs::remove_file(path);
            }
            out.code = 10;
            out.error = json!({"code":10,"message":"Token already exists, but its secret cannot be recovered. Revoke its ID and issue with a new key.","http_status":null,"retry_after_seconds":null});
            Ok((out, None))
        }
        TokenIssue::Issued { secret, .. } => {
            if let Some(file) = &mut file {
                if let Err(e) = super::secure_file::write(file, secret.expose()) {
                    out.code = 10;
                    out.error = json!({"code":10,"message":e.to_string(),"http_status":null,"retry_after_seconds":null});
                    return Ok((out, None));
                }
            }
            if args.show_secret {
                eprintln!("Sensitive output: stdout contains the new bearer credential.");
                let mut data: Value = out
                    .data
                    .as_ref()
                    .ok_or(Error::Protocol)?
                    .decode()
                    .map_err(|_| Error::Protocol)?;
                data["token"] = secret.expose().into();
                out.data = Outcome::data(data).data;
            }
            Ok((out, Some(secret)))
        }
    }
}
pub(super) async fn run(client: &Client, action: TokenAction) -> Result<Outcome, Error> {
    match action {
        TokenAction::List {
            state,
            cursor,
            limit,
        } => {
            let mut query = Vec::new();
            for (key, value) in [
                ("state", state),
                ("cursor", cursor),
                ("limit", limit.map(|v| v.to_string())),
            ] {
                if let Some(value) = value {
                    query.push((key.into(), value));
                }
            }
            Ok(Outcome::data(
                client.access_tokens().list(&query).await?.data,
            ))
        }
        TokenAction::Get { id } => {
            let r = client.access_tokens().get(&id).await?;
            Ok(Outcome {
                version: r.version.map(|v| v.as_str().to_owned()),
                ..Outcome::data(r.data)
            })
        }
        TokenAction::Current => Ok(Outcome::data(client.access_tokens().current().await?.data)),
        TokenAction::Create(args) => Ok(issue(client, args).await?.0),
        TokenAction::Revoke {
            id,
            current,
            if_match,
            yes,
        } => {
            if current == id.is_some() {
                return Err(Error::Invalid("choose either a token ID or --current"));
            }
            confirm(yes, id.as_deref().unwrap_or("current token"))?;
            if current {
                client.access_tokens().revoke_current().await?;
            } else {
                client
                    .access_tokens()
                    .revoke(
                        id.as_deref().ok_or(Error::Invalid("token ID required"))?,
                        ResourceVersion::parse(
                            if_match
                                .as_deref()
                                .ok_or(Error::Invalid("--if-match is required"))?,
                        )?,
                    )
                    .await?;
            }
            Ok(Outcome::data(json!({"revoked":true})))
        }
        TokenAction::Rotate {
            old_id,
            revoke_old,
            issue: args,
            if_match,
            yes,
        } => {
            if args.secret_out.is_none() {
                return Err(Error::Invalid(
                    "rotation requires --secret-out before revoking the old token",
                ));
            }
            let version = if revoke_old {
                Some(ResourceVersion::parse(if_match.as_deref().ok_or(
                    Error::Invalid("--revoke-old requires --if-match"),
                )?)?)
            } else {
                None
            };
            let session = client.session().await?.data;
            if revoke_old && !session.scopes.iter().any(|s| s == "access-token.revoke") {
                return Err(Error::Invalid("rotation requires access-token.revoke"));
            }
            if revoke_old {
                confirm(yes, &format!("rotate token {old_id}"))?;
            }
            let (mut out, secret) = issue(client, args).await?;
            if let Some(secret) = secret {
                let validation = client.with_credential(secret)?;
                let result = async {
                    if validation.session().await?.data.principal
                        != client.session().await?.data.principal
                    {
                        return Err(Error::Invalid("new token principal differs"));
                    }
                    validation.access_tokens().current().await?;
                    if let Some(version) = version {
                        client.access_tokens().revoke(&old_id, version).await?;
                    }
                    Ok(())
                }
                .await;
                if let Err(e) = result {
                    out.code = 8;
                    out.error = json!({"code":8,"message":format!("New token was saved; old token was not confirmed revoked: {e}"),"http_status":null,"retry_after_seconds":null});
                }
            }
            Ok(out)
        }
    }
}
pub(super) async fn auth(
    client: &Client,
    args: &ClientArgs,
    config: &super::config::Config,
    action: AuthAction,
) -> Result<Outcome, Error> {
    match action {
        AuthAction::Whoami | AuthAction::Status => Ok(Outcome::data(client.session().await?.data)),
        AuthAction::Login { credential_out } => {
            super::login::save(client, args, credential_out).await
        }
        AuthAction::Logout { revoke } => {
            if revoke {
                client.access_tokens().revoke_current().await?;
            }
            let context = super::config::context(args, config)?
                .ok_or(Error::Invalid("logout requires a context"))?;
            std::fs::remove_file(&context.token_file).map_err(|_| {
                Error::Invalid("remote revoke completed but local file removal failed")
            })?;
            Ok(Outcome::data(
                json!({"local_credential_removed":true,"remote_revoked":revoke}),
            ))
        }
    }
}
