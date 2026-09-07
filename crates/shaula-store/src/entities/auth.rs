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
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
