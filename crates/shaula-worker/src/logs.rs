use shaula_core::{
    error::{CoreError, CoreResult, ReasonCode},
    operation_log::*,
    worker::wire::*,
};
use uuid::Uuid;

impl crate::WorkerClient {
    async fn log(&self, message: LogMessage) -> CoreResult<LogResponse> {
        let request = LogRequest {
            request_id: Uuid::new_v4(),
            message,
        };
        let bytes = serde_json::to_vec(&request).map_err(|_| unavailable())?;
        if bytes.len() > 512 * 1024 {
            return Err(unavailable());
        }
        let response = self
            .http
            .post(format!("{}/logs", self.base))
            .bearer_auth(self.control.expose())
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(5))
            .body(bytes)
            .send()
            .await
            .map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        let bytes = crate::client::bounded(response, 512 * 1024)
            .await
            .map_err(|_| unavailable())?;
        serde_json::from_slice(&bytes).map_err(|_| unavailable())
    }
    async fn log_ack(&self, message: LogMessage) -> CoreResult<()> {
        match self.log(message).await? {
            LogResponse::Ack => Ok(()),
            _ => Err(unavailable()),
        }
    }
}

#[async_trait::async_trait]
impl OperationLogSink for crate::WorkerClient {
    async fn begin(&self, request: BeginInvocation) -> CoreResult<String> {
        match self.log(LogMessage::Begin(request)).await? {
            LogResponse::Invocation(id) => Ok(id),
            _ => Err(unavailable()),
        }
    }
    async fn append(&self, request: AppendLog) -> CoreResult<()> {
        self.log_ack(LogMessage::Append(request)).await
    }
    async fn command(&self, request: LogCommand) -> CoreResult<()> {
        self.log_ack(LogMessage::Command(request)).await
    }
    async fn finish(&self, request: FinishInvocation) -> CoreResult<()> {
        self.log_ack(LogMessage::Finish(request)).await
    }
}

#[async_trait::async_trait]
impl OperationLogReadPort for crate::WorkerClient {
    async fn list_invocations(&self, _: &str) -> CoreResult<Vec<Invocation>> {
        Err(unavailable())
    }
    async fn read_page(&self, _: &str, _: LogQuery) -> CoreResult<LogPage> {
        Err(unavailable())
    }
    async fn setup_projection(&self, _: &str) -> CoreResult<SetupProjection> {
        match self.log(LogMessage::Setup).await? {
            LogResponse::Setup(projection) => Ok(projection),
            _ => Err(unavailable()),
        }
    }
}

fn unavailable() -> CoreError {
    CoreError::new(
        ReasonCode::StorageUnavailable,
        "worker operation log unavailable",
    )
}
