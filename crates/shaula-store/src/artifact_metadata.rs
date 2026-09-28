//! Non-secret metadata of one stored archive for the management API.

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use shaula_core::registry::{TemplateArtifactMetadata, TemplateArtifactReference};

use crate::{Store, StoreResult};

/// Upper bound on Template Revision references returned for one archive.
const MAX_REFERENCES: usize = 200;

impl Store {
    pub async fn artifact_metadata(
        &self,
        digest: &str,
    ) -> StoreResult<Option<TemplateArtifactMetadata>> {
        let db = self.connection();
        let Some(row) = db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT length(archive) AS size_bytes,created_at FROM artifact_archives WHERE digest=?",
                [digest.into()],
            ))
            .await?
        else {
            return Ok(None);
        };
        let sources = db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT key FROM template_sources WHERE artifact_digest=? ORDER BY key",
                [digest.into()],
            ))
            .await?
            .iter()
            .map(|row| row.try_get::<String>("", "key"))
            .collect::<Result<Vec<_>, _>>()?;
        let mut revisions = db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT profile_key,revision,state FROM template_profile_revisions
                WHERE artifact_digest=? ORDER BY profile_key,revision LIMIT ?",
                [digest.into(), (MAX_REFERENCES as i64 + 1).into()],
            ))
            .await?
            .iter()
            .map(|row| {
                Ok(TemplateArtifactReference {
                    profile_key: row.try_get("", "profile_key")?,
                    revision: row.try_get("", "revision")?,
                    state: row.try_get("", "state")?,
                })
            })
            .collect::<Result<Vec<_>, sea_orm::DbErr>>()?;
        let revisions_truncated = revisions.len() > MAX_REFERENCES;
        revisions.truncate(MAX_REFERENCES);
        Ok(Some(TemplateArtifactMetadata {
            digest: digest.to_owned(),
            size_bytes: row.try_get("", "size_bytes")?,
            created_at: row.try_get("", "created_at")?,
            source_keys: sources,
            revisions,
            revisions_truncated,
        }))
    }
}
