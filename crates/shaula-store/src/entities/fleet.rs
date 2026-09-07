//! Fleet aggregate entities.

pub mod fleets {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "fleets")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
        pub incarnation: String,
        pub desired_revision: i64,
        pub observed_revision: i64,
        pub mutation_fence: i64,
        pub deletion_marker: bool,
        pub phase: String,
        pub tombstone: bool,
        pub last_condition_reason: Option<String>,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod fleet_revisions {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "fleet_revisions")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub fleet_key: String,
        pub incarnation: String,
        pub revision: i64,
        pub spec_json: String,
        pub template_profile_key: Option<String>,
        pub template_revision: Option<i64>,
        pub template_artifact_digest: Option<String>,
        pub template_attestation_id: Option<String>,
        pub auth_desired_profile_key: String,
        pub auth_desired_revision: i64,
        pub inputs_digest: String,
        pub actor: Option<String>,
        pub created_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod fleet_auth_handoffs {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "fleet_auth_handoffs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_key: String,
        pub desired_profile_key: String,
        pub desired_revision: i64,
        pub observed_profile_key: Option<String>,
        pub observed_revision: Option<i64>,
        pub state: String,
        pub cleanup_only: bool,
        pub attempts: i64,
        pub next_retry_at: Option<i64>,
        pub lease_owner: Option<String>,
        pub lease_expires_at: Option<i64>,
        pub reason: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod fleet_changes {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "fleet_changes")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub fleet_key: String,
        pub revision: i64,
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
