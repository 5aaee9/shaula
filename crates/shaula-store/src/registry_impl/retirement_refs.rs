//! Execution references, not historical rows, prevent logical retirement.
//! Called only while holding the same SQLite writer as the terminal commit.

use crate::store::StoreResult;
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbBackend, Statement};

pub(super) async fn in_use(
    tx: &DatabaseTransaction,
    key: &str,
    template: bool,
    now: i64,
) -> StoreResult<bool> {
    // A terminal Generation is not by itself proof of worker quiescence.
    // Keep open effects, worker leases and HTTP-state capabilities/locks.
    let common = "WITH live_generations AS (
        SELECT g.* FROM runner_generations g WHERE g.state!='Destroyed'
        OR EXISTS(SELECT 1 FROM runner_operations o WHERE o.generation_id=g.id
            AND (o.state IN ('Pending','Starting','ApplyStarting','BootstrapStarting','Running','Blocked')
                OR (o.lease_owner IS NOT NULL AND (o.lease_expires_at IS NULL OR o.lease_expires_at>?2))))
        OR EXISTS(SELECT 1 FROM generation_http_state s WHERE s.generation_id=g.id
            AND (s.revoked=0 OR s.lock_id IS NOT NULL))
    ), live_revisions AS (
        SELECT r.* FROM fleet_revisions r JOIN fleets f ON f.key=r.fleet_key
        WHERE (f.tombstone=0 AND r.revision=(SELECT MAX(h.revision) FROM fleet_revisions h
            WHERE h.fleet_key=f.key AND h.revision<=f.desired_revision))
        OR EXISTS(SELECT 1 FROM fleet_changes c WHERE c.fleet_key=r.fleet_key AND c.revision=r.revision
            AND (c.state IN ('Pending','Running','Blocked') OR (c.lease_owner IS NOT NULL
                AND (c.lease_expires_at IS NULL OR c.lease_expires_at>?2))))
    ) SELECT EXISTS(";
    let refs = if template {
        "SELECT 1 FROM live_generations WHERE template_profile_key=?1
        UNION ALL SELECT 1 FROM live_revisions WHERE template_profile_key=?1
        UNION ALL SELECT 1 FROM fleet_revision_pool_members m JOIN live_revisions r
            ON r.fleet_key=m.fleet_key AND r.revision=m.fleet_revision WHERE m.template_profile_key=?1
        UNION ALL SELECT 1 FROM template_pool_members m JOIN live_revisions r
            ON r.template_pool_ref=m.pool_key AND r.template_pool_revision=m.pool_revision
            WHERE m.template_profile_key=?1
        UNION ALL SELECT 1 FROM template_pool_members m JOIN template_pools p
            ON p.key=m.pool_key AND p.desired_revision=m.pool_revision
            WHERE p.tombstone=0 AND m.template_profile_key=?1"
    } else {
        "SELECT 1 FROM live_revisions WHERE auth_desired_profile_key=?1
        UNION ALL SELECT 1 FROM live_generations g JOIN fleet_revisions r
            ON r.fleet_key=g.fleet_key AND r.revision=g.fleet_revision WHERE r.auth_desired_profile_key=?1
        UNION ALL SELECT 1 FROM live_generations g JOIN runner_operations o ON o.generation_id=g.id
            WHERE (o.kind='JitStarting' AND json_extract(o.provenance_json,'$.context.auth_profile_key')=?1)
                OR (o.kind='ForgejoRegistration' AND json_extract(o.provenance_json,'$.auth[0]')=?1)
        UNION ALL SELECT 1 FROM fleet_auth_handoffs h JOIN fleets f ON f.key=h.fleet_key
            WHERE (h.desired_profile_key=?1 OR h.observed_profile_key=?1)
            AND (f.tombstone=0 OR (h.lease_owner IS NOT NULL
                AND (h.lease_expires_at IS NULL OR h.lease_expires_at>?2)))
        UNION ALL SELECT 1 FROM fleet_session_auth a JOIN fleet_sessions s ON s.fleet_key=a.fleet_key
            WHERE a.profile_key=?1
        UNION ALL SELECT 1 FROM listener_messages m JOIN listener_acquisitions a
            ON a.fleet_key=m.fleet_key AND a.epoch=m.epoch AND a.message_id=m.message_id
            WHERE m.profile_key=?1 AND a.state IN ('Pending','AcquireStarting','Uncertain')"
    };
    let sql = format!(
        "{common}{refs}
        UNION ALL SELECT 1 FROM profile_changes c WHERE c.resource_kind=?3 AND c.profile_key=?1
            AND c.lease_owner IS NOT NULL AND (c.lease_expires_at IS NULL OR c.lease_expires_at>?2)
        ) AS in_use"
    );
    let kind = if template {
        "template_profile"
    } else {
        "github_auth_profile"
    };
    let row = tx
        .query_one(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            sql,
            [key.into(), now.into(), kind.into()],
        ))
        .await?
        .ok_or_else(|| {
            crate::store::StoreError::Corrupt("missing retirement reference verdict".into())
        })?;
    Ok(row.try_get("", "in_use")?)
}
