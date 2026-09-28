use shaula_core::{
    ports::TemplateRuntimePort,
    state_backend::{StateError, StateResult},
    worker::{wire::*, PROTOCOL_VERSION},
};
use std::{sync::Arc, time::Duration};

use crate::{intent::RemoteIntent, WorkerClient};

/// One process, one Claim, one Create at most. Restart recovery launches an
/// explicitly cleanup-only Worker and never replays the old Create directive.
pub async fn run(
    launch: LaunchEnvelope,
    client: Arc<WorkerClient>,
    runtime: Arc<dyn TemplateRuntimePort>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> StateResult<()> {
    launch.validate()?;
    client
        .ack(ControlMessage::Handshake {
            protocol: PROTOCOL_VERSION,
        })
        .await?;
    if !launch.cleanup_only {
        let result = runtime
            .prepare_create(
                &launch.workspace,
                &launch.artifact,
                &launch.artifact_digest,
                launch.operation_timeout,
            )
            .await;
        let failed = result.is_err();
        client.ack(ControlMessage::Prepared(result)).await?;
        if failed {
            return Err(StateError::Unavailable);
        }
    } else {
        client.ack(ControlMessage::Prepared(Ok(()))).await?;
    }
    let sink = Arc::new(RemoteIntent(client.clone()));
    let mut created = launch.cleanup_only;
    let mut last_completed = None;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        let response = tokio::select! {
            response = client.call(ControlMessage::Poll) => response?,
            _ = shutdown.changed() => return Ok(()),
        };
        let ControlResponse::Desired(directive) = response else {
            return Err(StateError::Invalid);
        };
        match directive {
            Directive::Wait => {}
            Directive::Quiesce => return Ok(()),
            Directive::Complete(receipt) => {
                if receipt.claim != launch.claim {
                    return Err(StateError::Conflict);
                }
                // Only a durable receipt permits cleanup. Reaping is owned by
                // the executor; this process never removes an arbitrary path.
                return Ok(());
            }
            Directive::Create(reference) => {
                if last_completed == Some(reference.id) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
                if created {
                    return Err(StateError::Conflict);
                }
                created = true;
                let material: CreateMaterial = client.material(&reference).await?;
                let result = runtime
                    .create(material.into_request(&launch, sink.clone())?)
                    .await;
                client
                    .ack(ControlMessage::CreateFinished {
                        material_id: reference.id,
                        result,
                    })
                    .await?;
                last_completed = Some(reference.id);
            }
            Directive::Destroy(reference) => {
                if last_completed == Some(reference.id) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
                if !created {
                    return Err(StateError::Conflict);
                }
                let material: DestroyMaterial = client.material(&reference).await?;
                let protected_input = material.protected_input.clone();
                let request = material.into_request(&launch, sink.clone())?;
                let preparation = if launch.cleanup_only {
                    runtime
                        .prepare_recovery(
                            &request,
                            protected_input
                                .as_deref()
                                .ok_or(StateError::Unavailable)?
                                .as_bytes(),
                        )
                        .await
                } else {
                    Ok(())
                };
                let result = match preparation {
                    Ok(()) => runtime.destroy(request).await,
                    Err(error) => Err(error),
                };
                client
                    .ack(ControlMessage::DestroyFinished {
                        material_id: reference.id,
                        result,
                    })
                    .await?;
                last_completed = Some(reference.id);
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(200)) => {},
            _ = shutdown.changed() => return Ok(()),
        }
    }
}
