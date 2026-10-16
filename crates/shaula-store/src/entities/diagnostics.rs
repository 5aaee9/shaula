//! Disposable decision diagnostics (m0026). Rows are projections only; the
//! authoritative ledgers never depend on them.

pub mod diagnostic_snapshots {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "diagnostic_snapshots")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub subject_key: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub lane: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub question: String,
        pub epoch: String,
        pub sequence: i64,
        pub observed_at: i64,
        pub payload: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
