//! Operation Log tables (m0013). Chunk text lives in files; rows carry only
//! the sanitized record and its integrity metadata.

pub mod operation_log_invocations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "operation_log_invocations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub generation_id: String,
        pub fleet_key: String,
        pub operation: String,
        pub ordinal: i64,
        pub record_json: String,
        pub sealed_at: Option<i64>,
        pub retained_bytes: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod operation_log_chunks {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "operation_log_chunks")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub invocation_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub sequence: i64,
        pub command_ordinal: i64,
        pub phase: String,
        pub stream: String,
        pub observed_at: i64,
        pub bytes: i64,
        pub digest: String,
        pub file_name: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
