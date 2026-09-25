use super::*;
use types::{Document, FinalizeReceipt, NormalMutation};
pub type Query = Vec<(String, String)>;

macro_rules! resource {
    ($name:ident,$accessor:ident,$path:literal) => {
        pub struct $name<'a>(&'a Client);
        impl Client {
            pub fn $accessor(&self) -> $name<'_> {
                $name(self)
            }
        }
        impl $name<'_> {
            pub async fn list(&self) -> Result<Resource<Document>, Error> {
                self.0.read(&["api", "v1", $path], &[]).await
            }
            pub async fn get(&self, key: &str) -> Result<Resource<Document>, Error> {
                self.0.read(&["api", "v1", $path, key], &[]).await
            }
            pub async fn put(
                &self,
                key: &str,
                body: &Document,
                options: MutationOptions,
            ) -> Result<MutationAttempt, Error> {
                self.0
                    .prepare(
                        Method::PUT,
                        &["api", "v1", $path, key],
                        body.raw(),
                        Some(options),
                        None,
                    )
                    .await
            }
            pub async fn retire(
                &self,
                key: &str,
                options: MutationOptions,
            ) -> Result<MutationAttempt, Error> {
                if !matches!(options.precondition, WritePrecondition::Match(_)) {
                    return Err(Error::Invalid("retirement requires a reviewed version"));
                }
                self.0
                    .prepare(
                        Method::DELETE,
                        &["api", "v1", $path, key],
                        "",
                        Some(options),
                        None,
                    )
                    .await
            }
        }
    };
}
resource!(Fleets, fleets, "fleets");
resource!(Templates, templates, "template-profiles");
resource!(Pools, pools, "template-pools");
resource!(AuthProfiles, auth_profiles, "github-auth-profiles");

impl Fleets<'_> {
    pub async fn status(&self, key: &str) -> Result<Resource<Document>, Error> {
        self.0
            .read(&["api", "v1", "fleets", key, "status"], &[])
            .await
    }
}
impl Templates<'_> {
    pub async fn sources(&self) -> Result<Resource<Document>, Error> {
        self.0.read(&["api", "v1", "template-sources"], &[]).await
    }
    pub async fn variables(&self, digest: &str) -> Result<Resource<Document>, Error> {
        self.0
            .read(
                &["api", "v1", "template-artifacts", digest, "variables"],
                &[],
            )
            .await
    }
    pub async fn revision(&self, key: &str, revision: i64) -> Result<Resource<Document>, Error> {
        self.0
            .read(
                &[
                    "api",
                    "v1",
                    "template-profiles",
                    key,
                    "revisions",
                    &revision.to_string(),
                ],
                &[],
            )
            .await
    }
    pub async fn input_contract(
        &self,
        key: &str,
        revision: i64,
    ) -> Result<Resource<Document>, Error> {
        self.0
            .read(
                &[
                    "api",
                    "v1",
                    "template-profiles",
                    key,
                    "revisions",
                    &revision.to_string(),
                    "input-contract",
                ],
                &[],
            )
            .await
    }
    pub async fn attestation(
        &self,
        key: &str,
        revision: i64,
        attestation: &str,
    ) -> Result<Resource<Document>, Error> {
        self.0
            .read(
                &[
                    "api",
                    "v1",
                    "template-profiles",
                    key,
                    "revisions",
                    &revision.to_string(),
                    "attestations",
                    attestation,
                ],
                &[],
            )
            .await
    }
    pub async fn attest(
        &self,
        key: &str,
        revision: i64,
        attestation: &str,
        body: &Document,
        options: MutationOptions,
    ) -> Result<MutationAttempt, Error> {
        if !matches!(options.precondition, WritePrecondition::CreateOnly) {
            return Err(Error::Invalid(
                "attestations are immutable and require create-only",
            ));
        }
        self.0
            .prepare(
                Method::PUT,
                &[
                    "api",
                    "v1",
                    "template-profiles",
                    key,
                    "revisions",
                    &revision.to_string(),
                    "attestations",
                    attestation,
                ],
                body.raw(),
                Some(options),
                None,
            )
            .await
    }
    pub async fn update(
        &self,
        key: &str,
        body: &Document,
        options: MutationOptions,
    ) -> Result<MutationAttempt, Error> {
        if !matches!(options.precondition, WritePrecondition::Match(_)) {
            return Err(Error::Invalid("update requires a reviewed version"));
        }
        self.0
            .prepare(
                Method::POST,
                &["api", "v1", "template-profiles", key, "updates"],
                body.raw(),
                Some(options),
                None,
            )
            .await
    }
    pub async fn upload(&self, digest: &str, bytes: Vec<u8>) -> Result<Resource<Document>, Error> {
        self.0.upload(digest, bytes).await
    }
}
impl AuthProfiles<'_> {
    pub async fn status(&self, key: &str) -> Result<Resource<Document>, Error> {
        self.0
            .read(&["api", "v1", "github-auth-profiles", key, "status"], &[])
            .await
    }
    pub async fn impact(&self, key: &str) -> Result<Resource<Document>, Error> {
        self.0
            .read(&["api", "v1", "github-auth-profiles", key, "impact"], &[])
            .await
    }
    pub async fn installation_link(&self, key: &str) -> Result<Resource<Document>, Error> {
        self.0
            .read(
                &[
                    "api",
                    "v1",
                    "github-auth-profiles",
                    key,
                    "installation-link",
                ],
                &[],
            )
            .await
    }
    pub async fn revision(&self, key: &str, revision: i64) -> Result<Resource<Document>, Error> {
        self.0
            .read(
                &[
                    "api",
                    "v1",
                    "github-auth-profiles",
                    key,
                    "revisions",
                    &revision.to_string(),
                ],
                &[],
            )
            .await
    }
    pub async fn policy_update(
        &self,
        key: &str,
        body: &Document,
        options: MutationOptions,
    ) -> Result<MutationAttempt, Error> {
        if !matches!(options.precondition, WritePrecondition::Match(_)) {
            return Err(Error::Invalid("policy update requires a reviewed version"));
        }
        self.0
            .prepare(
                Method::POST,
                &["api", "v1", "github-auth-profiles", key, "policy-updates"],
                body.raw(),
                Some(options),
                None,
            )
            .await
    }
}
impl Client {
    pub async fn jobs(&self, query: &Query) -> Result<Resource<Document>, Error> {
        self.read(&["api", "v1", "jobs"], query).await
    }
    pub async fn job(&self, id: &str) -> Result<Resource<Document>, Error> {
        self.read(&["api", "v1", "jobs", id], &[]).await
    }
    pub async fn generations(&self, query: &Query) -> Result<Resource<Document>, Error> {
        self.read(&["api", "v1", "generations"], query).await
    }
    pub async fn generation(&self, id: &str) -> Result<Resource<Document>, Error> {
        self.read(&["api", "v1", "generations", id], &[]).await
    }
    pub async fn invocations(&self, id: &str, query: &Query) -> Result<Resource<Document>, Error> {
        self.read(&["api", "v1", "generations", id, "invocations"], query)
            .await
    }
    pub async fn logs(&self, id: &str, query: &Query) -> Result<Resource<Document>, Error> {
        self.read(&["api", "v1", "invocations", id, "logs"], query)
            .await
    }
    pub async fn finalize(
        &self,
        id: &str,
        reason: &str,
        key: Option<String>,
    ) -> Result<MutationAttempt, Error> {
        if reason.trim().is_empty() || reason.len() > 4096 {
            return Err(Error::Invalid("finalize requires a reason of 1–4096 bytes"));
        }
        self.prepare(
            Method::POST,
            &["api", "v1", "generations", id, "finalize"],
            &serde_json::json!({"reason":reason}).to_string(),
            None,
            key,
        )
        .await
    }
    pub async fn execute_mutation(
        &self,
        attempt: &MutationAttempt,
    ) -> Result<Resource<NormalMutation>, Error> {
        self.execute(attempt).await
    }
    pub async fn execute_finalize(
        &self,
        attempt: &MutationAttempt,
    ) -> Result<Resource<FinalizeReceipt>, Error> {
        self.execute(attempt).await
    }
}
