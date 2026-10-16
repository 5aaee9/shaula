//! Generation-scoped Terraform HTTP state and the lifecycle worker ledger
//! (m0007, m0027). CHECK constraints in the migrations stay authoritative.

pub mod generation_http_state {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "generation_http_state")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub generation_id: String,
        pub worker_epoch: i64,
        pub worker_attempt: String,
        pub capability_hash: Vec<u8>,
        pub revoked: bool,
        pub create_started: bool,
        pub sealed: bool,
        pub revision: i64,
        pub state_bytes: Option<Vec<u8>>,
        pub lineage: Option<String>,
        pub serial: Option<i64>,
        pub lock_id: Option<String>,
        pub lock_info: Option<Vec<u8>>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod lifecycle_workers {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "lifecycle_workers")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub generation_id: String,
        pub control_hash: Vec<u8>,
        pub phase: String,
        pub process_identity: Option<String>,
        pub cleanup_only: bool,
        pub handover: Option<String>,
        pub active_command: Option<String>,
        pub cleanup_revision: Option<i64>,
        pub protected_input: Option<Vec<u8>>,
        pub resource_state_seen: bool,
        pub completion_request: Option<String>,
        pub completion_receipt: Option<String>,
        pub workspace_reaped: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod lifecycle_fences {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "lifecycle_fences")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub worker_attempt: String,
        pub generation_id: String,
        pub worker_epoch: i64,
        pub process_identity: Option<String>,
        pub orphan_lock: Option<Vec<u8>>,
        pub confirmed_fenced: bool,
        pub outcome: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod lifecycle_deployment {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "lifecycle_deployment")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub singleton: i64,
        pub deployment_id: String,
        pub format_version: i64,
        pub migration_required: bool,
        pub activated: bool,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
