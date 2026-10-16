//! Listener message ledger (m0011 and later): one row per acknowledged
//! GitHub message, plus the acquisitions and job facts it carried.

pub mod listener_messages {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "listener_messages")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_key: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub epoch: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub message_id: i64,
        pub payload_digest: String,
        pub incarnation: String,
        pub fleet_revision: i64,
        pub mutation_fence: i64,
        pub profile_key: String,
        pub auth_revision: i64,
        pub context_json: String,
        pub acked: i64,
        pub created_at: i64,
        pub updated_at: i64,
        pub payload_digest_version: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod listener_acquisitions {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "listener_acquisitions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_key: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub epoch: i64,
        pub message_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub runner_request_id: i64,
        pub state: String,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod listener_job_observations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "listener_job_observations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_key: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub epoch: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub message_id: i64,
        #[sea_orm(primary_key, auto_increment = false)]
        pub observation_kind: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub runner_request_id: i64,
        pub job_id: String,
        pub runner_name: Option<String>,
        pub observed_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
