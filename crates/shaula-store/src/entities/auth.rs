//! GitHub Auth Profile aggregate entities. Credential plaintext bytes live
//! in `credential_bytes` by accepted design (ADR-0007/0009).

pub mod github_auth_profiles {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "github_auth_profiles")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
        pub incarnation: String,
        pub desired_revision: i64,
        pub active_revision: Option<i64>,
        pub observed_revision: Option<i64>,
        pub status: String,
        pub deletion_requested: bool,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod github_auth_profile_revisions {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "github_auth_profile_revisions")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub profile_key: String,
        pub revision: i64,
        pub kind: String,
        pub app_id: Option<String>,
        pub installation_id: Option<i64>,
        pub pat_principal: Option<String>,
        pub allowlist_json: String,
        pub credential_bytes: Vec<u8>,
        pub state: String,
        pub reason: Option<String>,
        pub created_at: i64,
        /// 1 = legacy single-installation, 2 = multi-account policy.
        pub schema_version: i64,
        /// Canonical TargetPolicy JSON (v2 only).
        pub policy_json: Option<String>,
        /// Validation snapshot: dependent-set fingerprint + checked fleet
        /// identities of the promotion gate (spec 0011 §4.1).
        pub validation_snapshot_json: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod github_auth_revision_bindings {
    use sea_orm::entity::prelude::*;

    /// One frozen Account Binding of a validated v2 revision.
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "github_auth_revision_bindings")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub profile_key: String,
        pub revision: i64,
        pub account_id: i64,
        pub account_kind: String,
        pub login: String,
        pub installation_id: i64,
        pub repository_selection: String,
        pub validated_at_ms: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod fleet_auth_contexts {
    use sea_orm::entity::prelude::*;

    /// Desired/observed exact Resolved Auth Context of one fleet.
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "fleet_auth_contexts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_key: String,
        pub desired_profile_key: Option<String>,
        pub desired_revision: Option<i64>,
        pub desired_fence: Option<i64>,
        pub desired_context_json: Option<String>,
        pub observed_profile_key: Option<String>,
        pub observed_revision: Option<i64>,
        pub observed_context_json: Option<String>,
        pub state: String,
        pub reason: Option<String>,
        pub attempts: i64,
        pub next_retry_at: Option<i64>,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
