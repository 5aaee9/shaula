//! Shared cross-resource entities: idempotency records, append-only audit
//! and the reconcile outbox. A `resource_kind` discriminator separates
//! Fleet and Profile facts while keeping one durable shape.

pub mod idempotency_records {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "idempotency_records")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub resource_kind: String,
        pub resource_key: String,
        pub idempotency_key: String,
        pub request_hash: String,
        pub response_status: i32,
        pub response_body: Option<String>,
        pub created_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod audit_records {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "audit_records")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub seq: i64,
        pub resource_kind: String,
        pub action: String,
        pub actor: String,
        pub resource_key: String,
        pub revision: Option<i64>,
        pub outcome: String,
        pub detail_json: Option<String>,
        pub created_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod outbox {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "outbox")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub resource_kind: String,
        pub topic: String,
        pub payload: String,
        pub created_at: i64,
        pub processed_at: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
