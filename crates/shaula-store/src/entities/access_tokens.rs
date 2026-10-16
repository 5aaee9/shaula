//! Personal access token tables (m0024). `record_json` is the full record;
//! the indexed columns mirror it for ownership, paging and expiry filters.

pub mod personal_access_tokens {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "personal_access_tokens")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub owner: String,
        pub created_at: i64,
        pub expires_at: i64,
        pub revoked_at: Option<i64>,
        pub last_used_at: Option<i64>,
        pub record_json: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod personal_token_issues {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "personal_token_issues")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub owner: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub idempotency_key: String,
        pub request_hash: String,
        pub token_id: String,
        pub created_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
