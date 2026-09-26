use super::*;
use types::diagnostics::{DiagnosticReportV1, SubjectKind};
impl Client {
    pub async fn diagnostics(
        &self,
        kind: SubjectKind,
        key: &str,
    ) -> Result<TypedResource<DiagnosticReportV1>, Error> {
        let path = match kind {
            SubjectKind::Fleet => "fleets",
            SubjectKind::Generation => "generations",
            SubjectKind::Job => "jobs",
            SubjectKind::Unknown => return Err(Error::Invalid("unknown diagnostic subject")),
        };
        let resource: Resource<types::Document> = self
            .read(&["api", "v1", path, key, "diagnostics"], &[])
            .await
            .map_err(|error| match error {
                Error::Protocol => Error::Unsupported,
                Error::Http {
                    status: 404,
                    ref code,
                    ..
                } if code != "DiagnosticsNotFound" => Error::Unsupported,
                error => error,
            })?;
        #[derive(serde::Deserialize)]
        struct Schema {
            #[serde(rename = "schemaVersion")]
            version: u32,
        }
        if resource
            .data
            .decode::<Schema>()
            .map_err(|_| Error::Unsupported)?
            .version
            != 1
        {
            return Err(Error::Unsupported);
        }
        let mut result = resource.typed::<DiagnosticReportV1>()?;
        if result.data.schema_version != 1 {
            return Err(Error::Unsupported);
        }
        if result.data.subject.kind != kind
            || result
                .data
                .subject
                .key
                .as_deref()
                .or(result.data.subject.id.as_deref())
                != Some(key)
        {
            return Err(Error::Protocol);
        }
        // An observation ID or a proxy-added ETag is never a mutation version.
        result.version = None;
        Ok(result)
    }
    pub async fn generation_diagnostics(
        &self,
        id: &str,
    ) -> Result<TypedResource<DiagnosticReportV1>, Error> {
        self.diagnostics(SubjectKind::Generation, id).await
    }
    pub async fn job_diagnostics(
        &self,
        id: &str,
    ) -> Result<TypedResource<DiagnosticReportV1>, Error> {
        self.diagnostics(SubjectKind::Job, id).await
    }
}
