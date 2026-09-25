use super::*;
use std::path::PathBuf;

pub(super) async fn save(
    client: &Client,
    args: &ClientArgs,
    destination: Option<PathBuf>,
) -> Result<Outcome, Error> {
    let session = client.session().await?.data;
    let pat = client
        .credential_for_storage()
        .expose()
        .starts_with("shaula_pat_v1_");
    if pat {
        client.access_tokens().current().await?;
    }
    let config_path = config::path(args)?;
    secure_file::safe_path(&config_path)?;
    let parent = config_path
        .parent()
        .ok_or(Error::Invalid("invalid config path"))?;
    secure_file::private_directory(parent)?;
    let mut config = config::load(args)?;
    let name = args
        .context
        .clone()
        .or(config.current_context.clone())
        .unwrap_or_else(|| "default".into());
    let destination = match destination {
        Some(path) => path,
        None => {
            let directory = parent.join("credentials");
            secure_file::private_directory(&directory)?;
            directory.join(format!("{}.token", uuid::Uuid::new_v4()))
        }
    };
    let destination =
        std::path::absolute(destination).map_err(|_| Error::Invalid("invalid credential path"))?;
    let mut file = secure_file::create(&destination)?;
    if let Err(error) = secure_file::write(&mut file, client.credential_for_storage().expose()) {
        drop(file);
        let _ = std::fs::remove_file(&destination);
        return Err(error);
    }
    config.contexts.insert(
        name.clone(),
        config::Context {
            server: client.origin().into(),
            credential_kind: if pat {
                CredentialKind::Pat
            } else {
                CredentialKind::Oidc
            },
            token_file: destination.clone(),
        },
    );
    config.current_context = Some(name.clone());
    let encoded = toml::to_string_pretty(&config)
        .map_err(|_| Error::Invalid("cannot encode client config"))?;
    let temporary = parent.join(format!(".{}.toml", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = secure_file::create(&temporary)?;
        secure_file::write(&mut file, &encoded)?;
        drop(file);
        std::fs::rename(&temporary, &config_path)
            .map_err(|_| Error::Invalid("could not atomically save client config"))
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        drop(file);
        let _ = std::fs::remove_file(&destination);
        return Err(error);
    }
    Ok(Outcome::data(
        json!({"principal":session.principal,"context":name,"credential_file":destination,"server":client.origin()}),
    ))
}
