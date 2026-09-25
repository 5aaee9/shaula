use super::args::*;
use shaula_client::{Client, Error, Secret};
use std::{collections::BTreeMap, io::Read, path::PathBuf};
#[derive(serde::Deserialize, serde::Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub current_context: Option<String>,
    #[serde(default)]
    pub contexts: BTreeMap<String, Context>,
}
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub server: String,
    pub credential_kind: CredentialKind,
    pub token_file: PathBuf,
}
pub fn path(args: &ClientArgs) -> Result<PathBuf, Error> {
    if let Some(path) = &args.client_config {
        return Ok(path.clone());
    }
    let root = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or(Error::Invalid(
            "use --client-config to select a configuration file",
        ))?;
    Ok(root.join("shaula/client.toml"))
}
pub fn load(args: &ClientArgs) -> Result<Config, Error> {
    let path = path(args)?;
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).map_err(|_| Error::Invalid("invalid client config")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(_) => Err(Error::Invalid("cannot read client config")),
    }
}
pub fn context<'a>(args: &ClientArgs, config: &'a Config) -> Result<Option<&'a Context>, Error> {
    let name = args.context.as_ref().or(config.current_context.as_ref());
    name.map(|n| {
        config
            .contexts
            .get(n)
            .ok_or(Error::Invalid("context does not exist"))
    })
    .transpose()
}
pub fn client(args: &ClientArgs, config: &Config) -> Result<Client, Error> {
    client_inner(args, config, false)
}
pub fn login_client(args: &ClientArgs, config: &Config) -> Result<Client, Error> {
    client_inner(args, config, true)
}
fn client_inner(args: &ClientArgs, config: &Config, login: bool) -> Result<Client, Error> {
    let context = match context(args, config) {
        Ok(c) => c,
        Err(_) if login => None,
        Err(e) => return Err(e),
    };
    let origin = args
        .server
        .as_deref()
        .or(context.map(|c| c.server.as_str()))
        .ok_or(Error::Invalid("specify --server or a context"))?;
    let raw = if args.token_stdin {
        let mut raw = String::new();
        std::io::stdin()
            .take(65_537)
            .read_to_string(&mut raw)
            .map_err(|_| Error::Invalid("cannot read credential stdin"))?;
        raw
    } else if let Some(file) = &args.token_file {
        super::secure_file::read(file)?
    } else if let Some(raw) = std::env::var_os("SHAULA_ACCESS_TOKEN") {
        raw.into_string()
            .map_err(|_| Error::Invalid("invalid credential environment"))?
    } else if let Some(context) = context {
        super::secure_file::read(&context.token_file)?
    } else {
        return Err(Error::Invalid(
            "supply --token-stdin, --token-file or SHAULA_ACCESS_TOKEN",
        ));
    };
    let raw = raw.trim_end_matches(['\r', '\n']).to_owned();
    if raw.len() > 65_536 {
        return Err(Error::Invalid("credential is too large"));
    }
    let kind = args
        .credential_kind
        .or(context.map(|c| c.credential_kind))
        .unwrap_or(CredentialKind::Pat);
    if matches!(kind, CredentialKind::Pat) != raw.starts_with("shaula_pat_v1_") {
        return Err(Error::Invalid(
            "credential does not match --credential-kind",
        ));
    }
    if args.allow_loopback_http {
        Client::loopback(origin, Secret::new(raw))
    } else {
        Client::new(origin, Secret::new(raw))
    }
}
