//! Template Profile aggregate entities.

pub mod template_profiles {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "template_profiles")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
        pub incarnation: String,
        pub desired_revision: i64,
        pub active_revision: Option<i64>,
        pub observed_revision: Option<i64>,
        pub active_attestation_id: Option<String>,
        pub status: String,
        pub deletion_requested: bool,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod template_profile_revisions {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "template_profile_revisions")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub profile_key: String,
        pub revision: i64,
        pub artifact_digest: String,
        pub engine_ref: String,
        pub platform: Option<String>,
        pub bindings_contract: Option<String>,
        pub manifest_json: Option<String>,
        pub lock_digest: Option<String>,
        pub bindings_json: Option<String>,
        pub bindings_digest: Option<String>,
        pub fleet_input_policy_json: Option<String>,
        pub state: String,
        pub reason: Option<String>,
        pub created_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod template_conformance_attestations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "template_conformance_attestations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub profile_key: String,
        pub revision: i64,
        pub subject_json: String,
        pub subject_digest: String,
        pub result: String,
        pub evidence_digest: Option<String>,
        pub suite_name: Option<String>,
        pub suite_version: Option<String>,
        pub completed_at: i64,
        pub subject_verified: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod template_artifacts {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "template_artifacts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub digest: String,
        pub size_bytes: i64,
        pub state: String,
        pub reference_count: i64,
        pub created_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod profile_changes {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "profile_changes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub resource_kind: String,
        pub profile_key: String,
        pub revision: Option<i64>,
        pub kind: String,
        pub state: String,
        pub attempts: i64,
        pub next_retry_at: Option<i64>,
        pub lease_owner: Option<String>,
        pub lease_expires_at: Option<i64>,
        pub reason: Option<String>,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
