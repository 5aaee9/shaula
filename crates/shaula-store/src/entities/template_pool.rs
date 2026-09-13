//! Shared TemplatePool aggregate entities (spec 0037).

pub mod template_pools {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "template_pools")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
        pub incarnation: String,
        pub desired_revision: i64,
        pub observed_revision: i64,
        pub phase: String,
        pub deletion_marker: bool,
        pub tombstone: bool,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod template_pool_revisions {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "template_pool_revisions")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub pool_key: String,
        pub revision: i64,
        pub spec_json: String,
        pub failure_policy: String,
        pub actor: Option<String>,
        pub created_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod template_pool_members {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "template_pool_members")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub pool_key: String,
        pub pool_revision: i64,
        pub member_key: String,
        pub template_profile_key: String,
        pub template_revision: i64,
        pub template_artifact_digest: String,
        pub template_attestation_id: String,
        pub template_inputs_json: String,
        pub inputs_digest: String,
        pub weight: i64,
        pub max_runners: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
