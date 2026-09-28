use shaula_core::{
    ports::{ApplyClaim, ApplyIntentSink, PlanProvenance},
    worker::wire::ControlMessage,
};
use std::sync::Arc;

pub(crate) struct RemoteIntent(pub Arc<crate::WorkerClient>);

#[async_trait::async_trait]
impl ApplyIntentSink for RemoteIntent {
    async fn persist_apply_starting(&self, proof: &PlanProvenance) -> Result<ApplyClaim, String> {
        self.0
            .ack(ControlMessage::ApplyStarting(proof.clone()))
            .await
            .map_err(|e| e.to_string())?;
        // The daemon retains the real Fleet guard until explicit spawn ACK.
        Ok(Box::new(()))
    }

    async fn authorize_bootstrap(&self, proof: &PlanProvenance) -> Result<ApplyClaim, String> {
        self.0
            .ack(ControlMessage::BootstrapStarting(proof.clone()))
            .await
            .map_err(|e| e.to_string())?;
        Ok(Box::new(()))
    }

    async fn spawn_handover(&self, proof: &PlanProvenance) -> Result<(), String> {
        self.0
            .ack(ControlMessage::Spawned(proof.clone()))
            .await
            .map_err(|e| e.to_string())
    }

    async fn command_ended(&self, proof: &PlanProvenance) -> Result<(), String> {
        self.0
            .ack(ControlMessage::CommandEnded(proof.clone()))
            .await
            .map_err(|e| e.to_string())
    }

    async fn bootstrap_ended(&self, proof: &PlanProvenance) -> Result<(), String> {
        self.0
            .ack(ControlMessage::BootstrapEnded(proof.clone()))
            .await
            .map_err(|e| e.to_string())
    }
}
