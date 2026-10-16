//! Jobs read-model tables (m0012, m0021, m0022, m0031). Job metadata is
//! retained evidence only; nothing here drives Runner lifecycle state.

pub mod workflow_jobs {
    use sea_orm::entity::prelude::*;

    /// GitHub listener job records.
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "workflow_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub scope_key: String,
        pub fleet_key: String,
        pub fleet_incarnation: String,
        pub scale_set_id: i64,
        pub protocol_job_id: String,
        pub summary_json: String,
        pub status: String,
        pub repository: Option<String>,
        pub job_name: Option<String>,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod workflow_job_observations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "workflow_job_observations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub scope_key: String,
        pub job_record_id: Option<String>,
        pub protocol_job_id: String,
        pub runner_request_id: i64,
        pub runner_id: Option<i64>,
        pub data_json: String,
        pub observed_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod workflow_generation_identity {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "workflow_generation_identity")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub generation_id: String,
        pub scope_key: String,
        pub fleet_incarnation: String,
        pub github_runner_id: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod forgejo_job_polls {
    use sea_orm::entity::prelude::*;

    /// Forgejo snapshot polls and the persisted history-read budget (m0022).
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "forgejo_job_polls")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub scope_key: String,
        pub observed_at: i64,
        pub attempted_at: i64,
        pub failed: bool,
        pub enrichment_attempted_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod forgejo_workflow_jobs {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "forgejo_workflow_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub scope_key: String,
        pub fleet_key: String,
        pub fleet_incarnation: String,
        pub protocol_job_id: String,
        pub summary_json: String,
        pub status: String,
        pub repository: Option<String>,
        pub job_name: Option<String>,
        pub in_snapshot: bool,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod forgejo_job_observations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "forgejo_job_observations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub job_record_id: String,
        pub data_json: String,
        pub observed_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod gitlab_job_polls {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "gitlab_job_polls")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub scope_key: String,
        pub enrichment_attempted_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod gitlab_workflow_jobs {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "gitlab_workflow_jobs")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub scope_key: String,
        pub fleet_key: String,
        pub fleet_incarnation: String,
        pub protocol_job_id: String,
        pub summary_json: String,
        pub status: String,
        pub repository: Option<String>,
        pub job_name: Option<String>,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod gitlab_job_generations {
    use sea_orm::entity::prelude::*;

    /// Append-only Verified links: triggers reject updates, and deletes
    /// while the linked job row still exists.
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "gitlab_job_generations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub job_record_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub generation_id: String,
        pub runner_id: i64,
        pub observed_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod gitlab_job_observations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "gitlab_job_observations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub job_record_id: String,
        pub data_json: String,
        pub observed_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
