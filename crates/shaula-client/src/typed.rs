use super::*;
use serde::Serialize;

/// Retains the original JSON alongside the typed projection, including future fields.
#[derive(Debug)]
pub struct TypedResource<T> {
    pub data: T,
    pub raw: types::Document,
    pub version: Option<ResourceVersion>,
}
impl Resource<types::Document> {
    pub fn typed<T: serde::de::DeserializeOwned>(self) -> Result<TypedResource<T>, Error> {
        let data = self.data.decode().map_err(|_| Error::Protocol)?;
        Ok(TypedResource {
            data,
            raw: self.data,
            version: self.version,
        })
    }
}
fn query(value: &impl Serialize) -> Result<Query, Error> {
    let value = serde_json::to_value(value).map_err(|_| Error::Invalid("invalid query"))?;
    Ok(value
        .as_object()
        .ok_or(Error::Invalid("invalid query"))?
        .iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| {
            (
                k.clone(),
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string()),
            )
        })
        .collect())
}
impl Client {
    pub async fn list_jobs(
        &self,
        options: &types::JobsQuery,
    ) -> Result<TypedResource<types::JobsPage>, Error> {
        self.jobs(&query(options)?).await?.typed()
    }
    pub async fn get_job(&self, id: &str) -> Result<TypedResource<types::JobDetail>, Error> {
        self.job(id).await?.typed()
    }
    pub async fn list_generations(
        &self,
        options: &types::GenerationsQuery,
    ) -> Result<TypedResource<types::GenerationsPage>, Error> {
        self.generations(&query(options)?).await?.typed()
    }
    pub async fn get_generation(
        &self,
        id: &str,
    ) -> Result<TypedResource<types::GenerationDetail>, Error> {
        self.generation(id).await?.typed()
    }
    pub async fn list_invocations(
        &self,
        id: &str,
        options: &types::InvocationQuery,
    ) -> Result<TypedResource<types::InvocationsPage>, Error> {
        self.invocations(id, &query(options)?).await?.typed()
    }
    pub async fn read_logs(
        &self,
        id: &str,
        options: &types::LogQuery,
    ) -> Result<TypedResource<types::LogPage>, Error> {
        self.logs(id, &query(options)?).await?.typed()
    }
}
impl Fleets<'_> {
    pub async fn get_typed(&self, key: &str) -> Result<TypedResource<types::Fleet>, Error> {
        self.get(key).await?.typed()
    }
    pub async fn put_typed(
        &self,
        key: &str,
        body: &types::FleetPut,
        options: MutationOptions,
    ) -> Result<MutationAttempt, Error> {
        self.put(key, &document(body)?, options).await
    }
}
impl Pools<'_> {
    pub async fn put_typed(
        &self,
        key: &str,
        body: &types::PoolPut,
        options: MutationOptions,
    ) -> Result<MutationAttempt, Error> {
        self.put(key, &document(body)?, options).await
    }
}
impl Templates<'_> {
    pub async fn publish(
        &self,
        key: &str,
        body: &types::TemplatePublish,
        options: MutationOptions,
    ) -> Result<MutationAttempt, Error> {
        self.put(key, &document(body)?, options).await
    }
    pub async fn update_typed(
        &self,
        key: &str,
        body: &types::TemplateUpdate,
        options: MutationOptions,
    ) -> Result<MutationAttempt, Error> {
        self.update(key, &document(body)?, options).await
    }
}
impl AuthProfiles<'_> {
    pub async fn publish(
        &self,
        key: &str,
        body: &types::AuthPublish,
        options: MutationOptions,
    ) -> Result<MutationAttempt, Error> {
        self.put(key, body.document(), options).await
    }
    pub async fn update_policy(
        &self,
        key: &str,
        body: &types::PolicyUpdate,
        options: MutationOptions,
    ) -> Result<MutationAttempt, Error> {
        self.policy_update(key, &document(body)?, options).await
    }
}
fn document(value: &impl Serialize) -> Result<types::Document, Error> {
    types::Document::from_serializable(value).map_err(|_| Error::Invalid("invalid request"))
}
