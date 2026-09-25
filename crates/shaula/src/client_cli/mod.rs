pub mod args;
mod command_name;
mod config;
mod edit;
mod history;
mod login;
mod resources;
mod secure_file;
mod tokens;
use args::*;
use serde_json::{json, Value};
use shaula_client::{ChangeKind, Client, Error, MutationAttempt};
use std::io::IsTerminal;

pub struct Outcome {
    pub data: Option<shaula_client::types::Document>,
    pub receipt: Value,
    pub error: Value,
    pub code: i32,
    pub partial: bool,
    pub version: Option<String>,
}
impl Outcome {
    pub fn data(data: impl serde::Serialize) -> Self {
        Self {
            data: shaula_client::types::Document::from_serializable(data).ok(),
            receipt: Value::Null,
            error: Value::Null,
            code: 0,
            partial: false,
            version: None,
        }
    }
    pub fn error(e: Error) -> Self {
        let code = match &e {
            Error::Invalid(_) => 2,
            Error::Http { status: 401, .. } => 3,
            Error::Http { status: 403, .. } => 4,
            Error::Http {
                status: 409 | 412 | 428,
                ..
            } => 5,
            Error::Http {
                status: 404 | 410, ..
            } => 6,
            Error::Http {
                status: 400 | 422, ..
            } => 7,
            Error::Tracking(_) => 8,
            Error::Transport | Error::Protocol | Error::Http { .. } => 9,
            _ => 1,
        };
        let (status, retry) = match &e {
            Error::Http {
                status,
                retry_after_seconds,
                ..
            } => (Some(*status), *retry_after_seconds),
            _ => (None, None),
        };
        Self {
            data: None,
            receipt: Value::Null,
            error: json!({"code":code,"message":e.to_string(),"http_status":status,"retry_after_seconds":retry}),
            code,
            partial: false,
            version: None,
        }
    }
}
pub fn output(args: &ClientArgs, command: &str, outcome: &Outcome) {
    #[derive(serde::Serialize)]
    struct Envelope<'a> {
        schema_version: u8,
        command: &'a str,
        data: &'a Option<shaula_client::types::Document>,
        metadata: Value,
        receipt: &'a Value,
        error: &'a Value,
    }
    let envelope = Envelope {
        schema_version: 1,
        command,
        data: &outcome.data,
        metadata: json!({"partial":outcome.partial,"version":outcome.version}),
        receipt: &outcome.receipt,
        error: &outcome.error,
    };
    let json = matches!(args.output, Some(Output::Json))
        || (args.output.is_none() && !std::io::stdout().is_terminal());
    if json {
        if let Ok(encoded) = serde_json::to_string(&envelope) {
            println!("{encoded}");
        }
    } else {
        println!("FIELD\tVALUE");
        if let Some(map) = serde_json::to_value(&envelope)
            .ok()
            .and_then(|v| v.as_object().cloned())
        {
            for (key, value) in map {
                println!("{key}\t{value}");
            }
        }
    }
}
pub fn event(event: &str, data: Value, partial: bool) {
    println!(
        "{}",
        json!({"schema_version":1,"event":event,"data":data,"metadata":{"partial":partial}})
    );
}
pub fn confirm(yes: bool, target: &str) -> Result<(), Error> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(Error::Invalid(
            "noninteractive destructive operations require --yes",
        ));
    }
    eprintln!(
        "Confirm {}? Type yes:",
        serde_json::to_string(target).map_err(|_| Error::Protocol)?
    );
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|_| Error::Invalid("confirmation unavailable"))?;
    if line.trim() == "yes" {
        Ok(())
    } else {
        Err(Error::Invalid("operation cancelled before submission"))
    }
}
pub async fn mutation(
    client: &Client,
    attempt: MutationAttempt,
    kind: ChangeKind,
    key: &str,
    wait: bool,
    timeout: u64,
) -> Outcome {
    let idempotency_key = attempt.idempotency_key().to_owned();
    match client.execute_mutation(&attempt).await {
        Err(e) => {
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
                out.receipt = json!({"outcome":"uncertain","idempotency_key":idempotency_key,"resource_key":key});
            }
            out
        }
        Ok(result) => {
            let receipt = json!({"outcome":if result.data.no_op {"no_op"} else {"accepted"},"resource_kind":kind,"resource_key":key,"change":if result.data.no_op {Value::Null} else {json!({"kind":kind,"id":result.data.change_id})},"idempotency_key":idempotency_key,"version":result.version.map(|v|v.as_str().to_owned())});
            let mut out = Outcome::data(&result.data);
            out.receipt = receipt;
            if wait && !result.data.no_op {
                let waited = tokio::select! { r=client.wait(kind,&result.data.change_id,std::time::Duration::from_secs(timeout))=>r,_=tokio::signal::ctrl_c()=>Err(Error::Tracking("cancelled; server operation continues".into())) };
                match waited {
                    Ok(change) => out.data = Outcome::data(change).data,
                    Err(e) => {
                        let failed = Outcome::error(e);
                        out.error = failed.error;
                        out.code = 8;
                    }
                }
            }
            out
        }
    }
}

pub async fn run(args: ClientArgs, command: RemoteCommand) -> i32 {
    let name = command_name::name(&command);
    let streaming = command_name::streaming(&command);
    let result = dispatch(&args, command).await;
    let out = result.unwrap_or_else(Outcome::error);
    if streaming {
        if out.code != 0 && out.code != 130 {
            event("error", out.error.clone(), true);
        }
        event(
            "end",
            json!({"reason":if out.code == 130 {"cancelled"} else if out.code != 0 {"error"} else {"timeout"},"exit_code":out.code}),
            out.partial || out.code != 0,
        );
    } else {
        output(&args, &name, &out);
    }
    out.code
}
async fn dispatch(args: &ClientArgs, command: RemoteCommand) -> Result<Outcome, Error> {
    let config = config::load(args)?;
    if matches!(command, RemoteCommand::Contexts { .. }) {
        return Ok(Outcome::data(config));
    }
    if let RemoteCommand::Auth {
        action: AuthAction::Logout { revoke: false },
    } = &command
    {
        let context =
            config::context(args, &config)?.ok_or(Error::Invalid("logout requires a context"))?;
        let _ = secure_file::read(&context.token_file)?;
        std::fs::remove_file(&context.token_file)
            .map_err(|_| Error::Invalid("cannot remove local credential"))?;
        return Ok(Outcome::data(
            json!({"local_credential_removed":true,"remote_revoked":false}),
        ));
    }
    let client = if matches!(
        command,
        RemoteCommand::Auth {
            action: AuthAction::Login { .. }
        }
    ) {
        config::login_client(args, &config)?
    } else {
        config::client(args, &config)?
    };
    match command {
        RemoteCommand::Fleets { action } => resources::run(&client, "fleets", action).await,
        RemoteCommand::Templates { action } => resources::run(&client, "templates", action).await,
        RemoteCommand::Pools { action } => resources::run(&client, "pools", action).await,
        RemoteCommand::AuthProfiles { action } => {
            resources::run(&client, "auth-profiles", action).await
        }
        RemoteCommand::Jobs { action } => history::run(&client, "jobs", action).await,
        RemoteCommand::Generations { action } => history::run(&client, "generations", action).await,
        RemoteCommand::Invocations { action } => history::run(&client, "invocations", action).await,
        RemoteCommand::Logs { action } => history::logs(&client, action).await,
        RemoteCommand::Changes { action } => match action {
            ChangeAction::Get { id, kind } => Ok(Outcome::data(
                client.change(ChangeKind::parse(&kind)?, &id).await?.data,
            )),
            ChangeAction::Wait { id, kind, timeout } => Ok(Outcome::data(
                client
                    .wait(
                        ChangeKind::parse(&kind)?,
                        &id,
                        std::time::Duration::from_secs(timeout),
                    )
                    .await?,
            )),
        },
        RemoteCommand::Tokens { action } => tokens::run(&client, action).await,
        RemoteCommand::Auth { action } => tokens::auth(&client, args, &config, action).await,
        RemoteCommand::Health { kind } => Ok(Outcome::data(
            json!({"healthy":if kind=="live" {client.live().await?} else {client.ready().await?}}),
        )),
        RemoteCommand::Contexts { .. } => Err(Error::Invalid("invalid command")),
    }
}
