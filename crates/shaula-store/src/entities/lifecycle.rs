//! Runner lifecycle ledger entities. Every runner row is namespaced by its
//! Fleet Key; no platform object columns exist in this schema.

pub mod runner_generations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "runner_generations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub fleet_key: String,
        pub runner_name: String,
        pub generation_name: String,
        pub fleet_revision: i64,
        pub template_profile_key: String,
        pub template_revision: i64,
        pub template_artifact_digest: String,
        pub attestation_id: String,
        pub inputs_digest: String,
        pub state: String,
        pub subphase: Option<String>,
        pub jit_phase: Option<String>,
        pub github_runner_id: Option<i64>,
        pub workspace_path: String,
        pub shaula_result_json: Option<String>,
        pub shaula_result_digest: Option<String>,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod forgejo_runner_identities {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "forgejo_runner_identities")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub generation_id: String,
        pub runner_id: i64,
        pub runner_uuid: String,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod runner_operations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "runner_operations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub generation_id: String,
        pub kind: String,
        pub state: String,
        pub attempts: i64,
        pub next_retry_at: Option<i64>,
        pub saved_plan_path: Option<String>,
        pub saved_plan_digest: Option<String>,
        pub provenance_json: Option<String>,
        pub lease_owner: Option<String>,
        pub lease_expires_at: Option<i64>,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod fleet_sessions {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "fleet_sessions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_key: String,
        pub session_id: String,
        pub epoch: i64,
        pub scale_set_id: i64,
        pub message_queue_url: Option<String>,
        pub queue_token: Option<String>,
        pub last_message_id: i64,
        pub created_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod fleet_demand {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "fleet_demand")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_key: String,
        pub total_assigned_jobs: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod job_observations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "job_observations")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i64,
        pub fleet_key: String,
        pub message_id: i64,
        pub observation_kind: String,
        pub runner_request_id: i64,
        pub job_id: String,
        pub runner_name: Option<String>,
        pub observed_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod scale_set_state {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scale_set_state")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_key: String,
        pub scale_set_id: Option<i64>,
        pub owned_scale_set_id: Option<i64>,
        pub name: String,
        pub runner_group: String,
        pub fingerprint: String,
        pub state: String,
        pub attempt_id: Option<String>,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
