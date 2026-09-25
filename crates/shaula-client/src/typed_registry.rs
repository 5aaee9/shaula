use super::*;
macro_rules! projection {
    ($method:ident, $raw:ident, $ty:ty) => {
        pub async fn $method(&self, key: &str) -> Result<TypedResource<$ty>, Error> {
            self.$raw(key).await?.typed()
        }
    };
}
impl Fleets<'_> {
    pub async fn list_typed(&self) -> Result<TypedResource<types::FleetsList>, Error> {
        self.list().await?.typed()
    }
    projection!(status_typed, status, types::FleetStatus);
}
impl Pools<'_> {
    pub async fn list_typed(&self) -> Result<TypedResource<types::PoolsList>, Error> {
        self.list().await?.typed()
    }
    projection!(get_typed, get, types::Pool);
}
impl Templates<'_> {
    pub async fn list_typed(&self) -> Result<TypedResource<types::ProfilesList>, Error> {
        self.list().await?.typed()
    }
    projection!(get_typed, get, types::Profile);
    projection!(variables_typed, variables, types::TemplateVariables);
    pub async fn sources_typed(&self) -> Result<TypedResource<types::TemplateSources>, Error> {
        self.sources().await?.typed()
    }
    pub async fn revision_typed(
        &self,
        key: &str,
        revision: i64,
    ) -> Result<TypedResource<types::TemplateRevision>, Error> {
        self.revision(key, revision).await?.typed()
    }
    pub async fn input_contract_typed(
        &self,
        key: &str,
        revision: i64,
    ) -> Result<TypedResource<types::InputContract>, Error> {
        self.input_contract(key, revision).await?.typed()
    }
    pub async fn attestation_typed(
        &self,
        key: &str,
        revision: i64,
        id: &str,
    ) -> Result<TypedResource<types::Attestation>, Error> {
        self.attestation(key, revision, id).await?.typed()
    }
    pub async fn upload_typed(
        &self,
        digest: &str,
        bytes: Vec<u8>,
    ) -> Result<TypedResource<types::ArtifactUpload>, Error> {
        self.upload(digest, bytes).await?.typed()
    }
}
impl AuthProfiles<'_> {
    pub async fn list_typed(&self) -> Result<TypedResource<types::ProfilesList>, Error> {
        self.list().await?.typed()
    }
    projection!(get_typed, get, types::Profile);
    projection!(status_typed, status, types::ProfileStatus);
    projection!(impact_typed, impact, types::ProfileImpact);
    projection!(
        installation_link_typed,
        installation_link,
        types::AuthInstallationLink
    );
    pub async fn revision_typed(
        &self,
        key: &str,
        revision: i64,
    ) -> Result<TypedResource<types::AuthRevision>, Error> {
        self.revision(key, revision).await?.typed()
    }
}
