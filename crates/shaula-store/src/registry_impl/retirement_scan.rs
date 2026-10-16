//! Level-triggered, restart-safe logical retirement. No protected bytes are GC'd.

use super::{core_err, retirement_refs, SqliteControlPlane};
use crate::entities::{
    auth::github_auth_profiles as auth, template::profile_changes,
    template::template_profiles as template,
};
use crate::store::StoreResult;
use sea_orm::{
    sea_query::{Expr, SimpleExpr},
    ColumnTrait, EntityTrait, QueryFilter, QuerySelect,
};
use shaula_core::error::CoreResult;

/// The columns both profile tables share for logical retirement.
trait RetiringProfile: EntityTrait {
    const TEMPLATE: bool;
    const KIND: &'static str;
    const KEY: Self::Column;
    const DELETION_REQUESTED: Self::Column;
    const STATUS: Self::Column;
    const DESIRED_REVISION: Self::Column;
    const ACTIVE_REVISION: Self::Column;
    const OBSERVED_REVISION: Self::Column;
    const UPDATED_AT: Self::Column;
    /// Revision pointers cleared beyond active/observed on retirement.
    const ALSO_CLEARED: &'static [Self::Column];
}

impl RetiringProfile for template::Entity {
    const TEMPLATE: bool = true;
    const KIND: &'static str = "template_profile";
    const KEY: template::Column = template::Column::Key;
    const DELETION_REQUESTED: template::Column = template::Column::DeletionRequested;
    const STATUS: template::Column = template::Column::Status;
    const DESIRED_REVISION: template::Column = template::Column::DesiredRevision;
    const ACTIVE_REVISION: template::Column = template::Column::ActiveRevision;
    const OBSERVED_REVISION: template::Column = template::Column::ObservedRevision;
    const UPDATED_AT: template::Column = template::Column::UpdatedAt;
    const ALSO_CLEARED: &'static [template::Column] = &[template::Column::ActiveAttestationId];
}

impl RetiringProfile for auth::Entity {
    const TEMPLATE: bool = false;
    const KIND: &'static str = "github_auth_profile";
    const KEY: auth::Column = auth::Column::Key;
    const DELETION_REQUESTED: auth::Column = auth::Column::DeletionRequested;
    const STATUS: auth::Column = auth::Column::Status;
    const DESIRED_REVISION: auth::Column = auth::Column::DesiredRevision;
    const ACTIVE_REVISION: auth::Column = auth::Column::ActiveRevision;
    const OBSERVED_REVISION: auth::Column = auth::Column::ObservedRevision;
    const UPDATED_AT: auth::Column = auth::Column::UpdatedAt;
    const ALSO_CLEARED: &'static [auth::Column] = &[];
}

fn retiring<E: RetiringProfile>() -> SimpleExpr {
    E::DELETION_REQUESTED.eq(true).and(E::STATUS.eq("Retiring"))
}

impl SqliteControlPlane {
    pub(super) async fn scan_retirements(&self, now: i64) -> CoreResult<usize> {
        let completed = self.scan::<template::Entity>(now).await?;
        Ok(completed + self.scan::<auth::Entity>(now).await?)
    }

    async fn scan<E: RetiringProfile>(&self, now: i64) -> CoreResult<usize> {
        // Only keys are captured outside the transaction. All authority and
        // references are re-read after reserving the writer.
        let keys = E::find()
            .select_only()
            .column(E::KEY)
            .filter(retiring::<E>())
            .into_tuple::<String>()
            .all(self.store.connection())
            .await
            .map_err(|e| core_err(e.into()))?;
        let mut completed = 0;
        for key in keys {
            completed += usize::from(
                self.finish_retirement::<E>(&key, now)
                    .await
                    .map_err(core_err)?,
            );
        }
        Ok(completed)
    }

    async fn finish_retirement<E: RetiringProfile>(
        &self,
        key: &str,
        now: i64,
    ) -> StoreResult<bool> {
        let tx = self.store.begin().await?;
        let head = E::find()
            .select_only()
            .column(E::DESIRED_REVISION)
            .filter(E::KEY.eq(key))
            .filter(retiring::<E>())
            .into_tuple::<i64>()
            .one(&tx)
            .await?;
        let Some(revision) = head else {
            return Ok(false);
        };
        if retirement_refs::in_use(&tx, key, E::TEMPLATE, now).await? {
            return Ok(false);
        }
        // desired_revision remains only a historical high-water mark, never an
        // execution head: deletion_requested fences PUT/activation/references.
        let mut retire = E::update_many()
            .col_expr(E::STATUS, Expr::value("Retired"))
            .col_expr(E::ACTIVE_REVISION, Expr::value(Option::<i64>::None))
            .col_expr(E::OBSERVED_REVISION, Expr::value(Option::<i64>::None))
            .col_expr(E::UPDATED_AT, Expr::value(now))
            .filter(E::KEY.eq(key));
        for column in E::ALSO_CLEARED {
            retire = retire.col_expr(*column, Expr::value(Option::<String>::None));
        }
        retire.exec(&tx).await?;
        let settled = Expr::case(profile_changes::Column::Kind.eq("Retire"), "Succeeded")
            .finally("Superseded");
        profile_changes::Entity::update_many()
            .col_expr(profile_changes::Column::State, settled.into())
            .col_expr(
                profile_changes::Column::Reason,
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                profile_changes::Column::NextRetryAt,
                Expr::value(Option::<i64>::None),
            )
            .col_expr(
                profile_changes::Column::LeaseOwner,
                Expr::value(Option::<String>::None),
            )
            .col_expr(
                profile_changes::Column::LeaseExpiresAt,
                Expr::value(Option::<i64>::None),
            )
            .col_expr(profile_changes::Column::UpdatedAt, Expr::value(now))
            .filter(profile_changes::Column::ResourceKind.eq(E::KIND))
            .filter(profile_changes::Column::ProfileKey.eq(key))
            .filter(profile_changes::Column::Revision.lte(revision))
            .filter(profile_changes::Column::State.is_in(["Pending", "Running", "Blocked"]))
            .exec(&tx)
            .await?;
        self.store
            .audit_append(
                &tx,
                shaula_core::registry::AuditAppend {
                    authentication: Default::default(),
                    resource_kind: E::KIND.into(),
                    action: "retire".into(),
                    actor: "reconciler".into(),
                    resource_key: key.into(),
                    revision: Some(revision),
                    outcome: "succeeded".into(),
                    detail_json: None,
                    now,
                },
            )
            .await?;
        tx.commit().await?;
        Ok(true)
    }
}
