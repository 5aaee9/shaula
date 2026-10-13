//! GitLab Pool tables (m0030): kept apart from GitHub and Forgejo identities.

pub mod gitlab_runner_identities {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "gitlab_runner_identities")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub generation_id: String,
        pub runner_id: i64,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod gitlab_anchors {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "gitlab_anchors")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_key: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub fleet_revision: i64,
        pub anchor_name: String,
        pub runner_id: Option<i64>,
        pub state: String,
        pub created_at: i64,
        pub updated_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod gitlab_job_associations {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "gitlab_job_associations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub generation_id: String,
        pub job_id: i64,
        pub project_id: i64,
        pub pipeline_id: i64,
        pub observed_at: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
