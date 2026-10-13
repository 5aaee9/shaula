//! The Fleet-owned paused anchor: GitLab's only authorized proof of
//! compatible pending demand. One anchor per Fleet revision (tags may change
//! between revisions); its token is dropped on receipt, so no process can
//! ever run a job on it and any other anchor of this Fleet is safe to delete.

use shaula_core::error::{CoreResult, ReasonCode};
use shaula_core::gitlab::{ownership_note, GitlabRegistrationRole};
use shaula_core::ports::gitlab::{GitlabRunnerRef, GitlabRunnerSpec};
use shaula_core::ports::EffectOutcome;
use shaula_core::registry::GitlabAnchorState;

use super::{access_reason, GitlabPoolSupervisor, GitlabReconcileReport};

impl GitlabPoolSupervisor {
    fn anchor_name(&self, revision: i64) -> String {
        format!("{}anchor-r{revision}", self.runner_name_prefix)
    }

    pub(super) fn is_anchor(&self, runner: &GitlabRunnerRef) -> bool {
        runner.maintenance_note.as_deref()
            == Some(
                ownership_note(
                    &self.fleet_key,
                    GitlabRegistrationRole::Anchor,
                    &runner.description,
                )
                .as_str(),
            )
    }

    /// Converges to exactly one anchor for the current revision (none while
    /// deleting) and returns its ID when one is retained.
    pub(super) async fn reconcile_anchors(
        &self,
        runners: &[GitlabRunnerRef],
        deleting: bool,
        now: i64,
    ) -> CoreResult<Option<u64>> {
        let _claim = self.gates.acquire_claim(&self.fleet_key).await;
        let rows = self.lifecycle.gitlab_anchors(&self.fleet_key).await?;
        let anchors: Vec<&GitlabRunnerRef> = runners.iter().filter(|r| self.is_anchor(r)).collect();
        let revision = self.guard.desired_revision;
        let mut keep = None;
        if let Some(row) = rows
            .iter()
            .find(|row| !deleting && row.fleet_revision == revision)
        {
            let named: Vec<_> = anchors
                .iter()
                .filter(|r| r.description == row.anchor_name)
                .collect();
            match row.runner_id.and_then(|id| u64::try_from(id).ok()) {
                Some(id) if named.iter().any(|r| r.id == id) => keep = Some(id),
                // An unrecorded registration outcome: adopt a unique landing.
                None if named.len() == 1 => {
                    let id = named[0].id;
                    self.lifecycle
                        .gitlab_anchor_set(
                            &self.fleet_key,
                            revision,
                            i64::try_from(id).ok(),
                            GitlabAnchorState::Active,
                            now,
                        )
                        .await?;
                    keep = Some(id);
                }
                _ => {}
            }
        }
        let mut retained = Vec::new();
        for anchor in anchors.iter().filter(|r| Some(r.id) != keep) {
            if self.gitlab.delete_runner(anchor.id).await.is_err() {
                retained.push(anchor.description.as_str());
            }
        }
        for row in &rows {
            let kept = keep.is_some() && row.fleet_revision == revision && !deleting;
            if !kept && !retained.contains(&row.anchor_name.as_str()) {
                self.lifecycle
                    .gitlab_anchor_remove(&self.fleet_key, row.fleet_revision)
                    .await?;
            }
        }
        if deleting || keep.is_some() || !retained.is_empty() {
            return Ok(keep);
        }
        self.register_anchor(revision, now).await
    }

    async fn register_anchor(&self, revision: i64, now: i64) -> CoreResult<Option<u64>> {
        let name = self.anchor_name(revision);
        self.lifecycle
            .gitlab_anchor_begin(&self.fleet_key, revision, &name, now)
            .await?;
        let spec = GitlabRunnerSpec {
            maintenance_note: ownership_note(
                &self.fleet_key,
                GitlabRegistrationRole::Anchor,
                &name,
            ),
            description: name,
            role: GitlabRegistrationRole::Anchor,
            matching: self.matching.clone(),
        };
        let (id, state) = match self.gitlab.register_runner(&spec).await {
            // The token is dropped here; the anchor is never bootstrapped.
            Ok(EffectOutcome::Definite(registration)) => {
                (Some(registration.id), GitlabAnchorState::Active)
            }
            _ => (None, GitlabAnchorState::Ambiguous),
        };
        self.lifecycle
            .gitlab_anchor_set(
                &self.fleet_key,
                revision,
                id.and_then(|id| i64::try_from(id).ok()),
                state,
                now,
            )
            .await?;
        Ok(id)
    }

    /// Demand is the anchor's same-request proof or nothing: a failed or
    /// unproven read keeps the old demand without a fresh timestamp.
    pub(super) async fn observe_demand(
        &self,
        anchor: Option<u64>,
        deleting: bool,
        now: i64,
    ) -> CoreResult<GitlabReconcileReport> {
        let mut report = GitlabReconcileReport::default();
        if deleting {
            return Ok(report);
        }
        let proven = match anchor {
            None => Err(ReasonCode::OwnershipProofFailed),
            Some(id) => match self.gitlab.anchor_demand(id).await {
                Err(failure) => Err(access_reason(&failure)),
                Ok(demand) => match demand.proven_for(&self.matching) {
                    Some(pending) => Ok(pending),
                    None => {
                        // Drift is replaced, never repaired in place: an
                        // unpaused anchor counts as drift too (spec 0043
                        // §3.1). The next tick converges to a new anchor.
                        if demand.anchor_present
                            && (demand.anchor_matching != self.matching || !demand.anchor_paused)
                        {
                            let _ = self.gitlab.delete_runner(id).await;
                        }
                        Err(if demand.anchor_present && !demand.anchor_manageable {
                            ReasonCode::PermissionDenied
                        } else {
                            ReasonCode::OwnershipProofFailed
                        })
                    }
                },
            },
        };
        let _claim = self.gates.acquire_claim(&self.fleet_key).await;
        let current = self
            .current_head()
            .await?
            .is_some_and(|head| !head.deletion_marker);
        match proven {
            Ok(pending) if current => {
                self.lifecycle
                    .demand_snapshot(
                        &self.fleet_key,
                        i64::try_from(pending).unwrap_or(i64::MAX),
                        now,
                    )
                    .await?;
                report.pending_jobs = pending;
            }
            outcome => {
                report.pending_jobs = u64::try_from(
                    self.store
                        .demand_get(&self.fleet_key)
                        .await?
                        .unwrap_or(0)
                        .max(0),
                )
                .unwrap_or(0);
                report.stale_demand = true;
                report.reason = Some(outcome.err().unwrap_or(ReasonCode::OwnershipProofFailed));
            }
        }
        report.target = self.target(report.pending_jobs);
        Ok(report)
    }
}
