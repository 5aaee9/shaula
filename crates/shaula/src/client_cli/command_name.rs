use super::args::*;
pub(super) fn name(command: &RemoteCommand) -> String {
    let (group, action) = match command {
        RemoteCommand::Fleets { action } => ("fleets", resource(action)),
        RemoteCommand::Templates { action } => ("templates", resource(action)),
        RemoteCommand::Pools { action } => ("pools", resource(action)),
        RemoteCommand::AuthProfiles { action } => ("auth-profiles", resource(action)),
        RemoteCommand::Jobs { action } => ("jobs", history(action)),
        RemoteCommand::Generations { action } => ("generations", history(action)),
        RemoteCommand::Invocations { action } => ("invocations", history(action)),
        RemoteCommand::Logs { action } => (
            "logs",
            match action {
                LogAction::Read(_) => "read",
                LogAction::Download { .. } => "download",
            },
        ),
        RemoteCommand::Changes { action } => (
            "changes",
            match action {
                ChangeAction::Get { .. } => "get",
                ChangeAction::Wait { .. } => "wait",
            },
        ),
        RemoteCommand::Tokens { action } => (
            "tokens",
            match action {
                TokenAction::List { .. } => "list",
                TokenAction::Get { .. } => "get",
                TokenAction::Current => "current",
                TokenAction::Create(_) => "create",
                TokenAction::Rotate { .. } => "rotate",
                TokenAction::Revoke { .. } => "revoke",
            },
        ),
        RemoteCommand::Auth { action } => (
            "auth",
            match action {
                AuthAction::Whoami => "whoami",
                AuthAction::Status => "status",
                AuthAction::Login { .. } => "login",
                AuthAction::Logout { .. } => "logout",
            },
        ),
        RemoteCommand::Health { kind } => ("health", kind.as_str()),
        RemoteCommand::Contexts { action } => ("contexts", action.as_str()),
    };
    format!("{group}.{action}")
}
fn resource(action: &ResourceAction) -> &str {
    match action {
        ResourceAction::Explain { .. } => "explain",
        ResourceAction::List => "list",
        ResourceAction::Get { .. } => "get",
        ResourceAction::Status { .. } => "status",
        ResourceAction::Create(_) => "create",
        ResourceAction::Update(_) => "update",
        ResourceAction::Publish(_) => "publish",
        ResourceAction::Rotate(_) => "rotate",
        ResourceAction::PolicyUpdate(_) => "policy-update",
        ResourceAction::Retire(_) => "retire",
        ResourceAction::Edit { .. } => "edit",
        ResourceAction::Impact { .. } => "impact",
        ResourceAction::InstallationLink { .. } => "installation-link",
        ResourceAction::Sources { .. } => "sources.list",
        ResourceAction::Variables { .. } => "variables",
        ResourceAction::Revisions { action } => match action {
            RevisionAction::Get { .. } => "revisions.get",
            RevisionAction::Publish(_) => "revisions.publish",
        },
        ResourceAction::InputContract { .. } => "input-contract",
        ResourceAction::Attestations { action, .. } => {
            if action == "get" {
                "attestations.get"
            } else {
                "attestations.create"
            }
        }
        ResourceAction::Upload { .. } | ResourceAction::Artifacts { .. } => "artifacts.upload",
    }
}
fn history(action: &HistoryAction) -> &str {
    match action {
        HistoryAction::Explain { .. } => "explain",
        HistoryAction::List { .. } => "list",
        HistoryAction::Get { .. } => "get",
        HistoryAction::Finalize { .. } => "finalize",
    }
}
pub(super) fn streaming(command: &RemoteCommand) -> bool {
    matches!(
        command,
        RemoteCommand::Fleets {
            action: ResourceAction::Explain { watch: true, .. }
        } | RemoteCommand::Jobs {
            action: HistoryAction::Explain { watch: true, .. }
        } | RemoteCommand::Generations {
            action: HistoryAction::Explain { watch: true, .. }
        } | RemoteCommand::Fleets {
            action: ResourceAction::Status { watch: true, .. }
        } | RemoteCommand::AuthProfiles {
            action: ResourceAction::Status { watch: true, .. }
        } | RemoteCommand::Logs {
            action: LogAction::Read(LogArgs { follow: true, .. })
        } | RemoteCommand::Logs {
            action: LogAction::Download {
                args: LogArgs { follow: true, .. },
                ..
            }
        }
    )
}
