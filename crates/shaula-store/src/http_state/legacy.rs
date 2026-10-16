//! The legacy Generation projection that cutover commitments hash. Its JSON
//! shape is part of each commitment: keep keys, order and values exact.
use super::workers;
use crate::entities::http_state::lifecycle_imports as imports;
use crate::entities::lifecycle::{
    forgejo_runner_identities as forgejo, runner_generations as generations,
    runner_operations as operations,
};
use crate::query_ext::{col, probe, same};
use sea_orm::{
    sea_query::{Alias, Asterisk, Expr, Func, Query, SimpleExpr},
    ColumnTrait, EntityTrait, FromQueryResult, QuerySelect, Select,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use shaula_core::state_backend::{StateDocument, StateError, StateResult};

#[derive(FromQueryResult, Serialize)]
pub struct LegacyGeneration {
    pub id: String,
    pub fleet_key: String,
    pub workspace_path: String,
    pub template_artifact_digest: String,
    pub inputs_digest: String,
    pub state: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub effects: String,
    pub authority: String,
    pub create_result: Option<String>,
    pub resources_destroyed_at: Option<i64>,
}

impl LegacyGeneration {
    /// Imported bytes must be anchored to the daemon's retained successful
    /// Create, rather than certify their own ownership or completeness.
    pub fn state_matches_original(&self, state: &StateDocument) -> bool {
        super::create_proof::matches(self.create_result.as_deref(), state)
    }
    pub fn commitment(&self) -> StateResult<String> {
        Ok(format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                serde_json::to_vec(self).map_err(|_| StateError::Invalid)?
            ))
        ))
    }
}

fn json(function: &str, args: Vec<SimpleExpr>) -> SimpleExpr {
    Func::cust(Alias::new(function)).args(args).into()
}

/// Every Generation-owned authority fact, including its Forgejo identity.
fn authority() -> SimpleExpr {
    use generations::Column as G;
    let identity = Query::select()
        .expr(json(
            "json_array",
            [
                forgejo::Column::RunnerId,
                forgejo::Column::RunnerUuid,
                forgejo::Column::CreatedAt,
                forgejo::Column::UpdatedAt,
            ]
            .map(|c| col(c).into())
            .into(),
        ))
        .from(forgejo::Entity)
        .and_where(same(forgejo::Column::GenerationId, G::Id))
        .to_owned();
    let mut facts: Vec<SimpleExpr> = [
        G::RunnerName,
        G::GenerationName,
        G::FleetRevision,
        G::TemplateProfileKey,
        G::TemplateRevision,
        G::AttestationId,
        G::PoolMemberKey,
        G::Subphase,
        G::JitPhase,
        G::GithubRunnerId,
        G::ShaulaResultJson,
        G::ShaulaResultDigest,
        G::ProvisionedAt,
        G::ExpiryRequestedAt,
        G::ResourcesDestroyedAt,
    ]
    .map(|c| col(c).into())
    .into();
    facts.push(crate::query_ext::scalar(identity));
    json("json_array", facts)
}

/// Every operation of the Generation, in id order.
fn effects() -> SimpleExpr {
    use operations::Column as O;
    let o = Alias::new("o");
    let ordered = Query::select()
        .column(Asterisk)
        .from(operations::Entity)
        .and_where(same(O::GenerationId, generations::Column::Id))
        .order_by(O::Id, sea_orm::Order::Asc)
        .to_owned();
    let fields = [
        ("id", O::Id),
        ("kind", O::Kind),
        ("state", O::State),
        ("provenance", O::ProvenanceJson),
        ("updated_at", O::UpdatedAt),
        ("attempts", O::Attempts),
        ("plan", O::SavedPlanDigest),
        ("path", O::SavedPlanPath),
        ("owner", O::LeaseOwner),
        ("lease", O::LeaseExpiresAt),
        ("retry", O::NextRetryAt),
        ("created_at", O::CreatedAt),
    ]
    .into_iter()
    .flat_map(|(key, column)| [Expr::val(key).into(), Expr::col((o.clone(), column)).into()])
    .collect();
    let group = Query::select()
        .expr(json("json_group_array", vec![json("json_object", fields)]))
        .from_subquery(ordered, o.clone())
        .to_owned();
    crate::query_ext::scalar(group)
}

/// The commitment-bearing projection of Generations.
pub(super) fn select() -> Select<generations::Entity> {
    use generations::Column as G;
    generations::Entity::find()
        .select_only()
        .columns([
            G::Id,
            G::FleetKey,
            G::WorkspacePath,
            G::TemplateArtifactDigest,
            G::InputsDigest,
        ])
        .column_as(G::ShaulaResultJson, "create_result")
        .columns([
            G::ResourcesDestroyedAt,
            G::State,
            G::CreatedAt,
            G::UpdatedAt,
        ])
        .expr_as(authority(), "authority")
        .expr_as(effects(), "effects")
}

/// Live Generations neither admitted as workers nor classified by an import.
pub(super) fn unclassified() -> SimpleExpr {
    use generations::Column as G;
    let worker = probe(workers::Entity)
        .and_where(same(workers::Column::GenerationId, G::Id))
        .to_owned();
    let import = probe(imports::Entity)
        .and_where(same(imports::Column::GenerationId, G::Id))
        .to_owned();
    G::State
        .ne("Destroyed")
        .and(Expr::exists(worker).not())
        .and(Expr::exists(import).not())
}
