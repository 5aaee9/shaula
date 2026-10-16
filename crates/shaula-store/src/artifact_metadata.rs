//! Non-secret metadata of one stored archive for the management API.

use sea_orm::{
    sea_query::{Alias, Func},
    ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect,
};
use shaula_core::registry::{TemplateArtifactMetadata, TemplateArtifactReference};

use crate::artifact_library::{archives, sources};
use crate::entities::template::template_profile_revisions as revisions;
use crate::query_ext::col;
use crate::{Store, StoreResult};

/// Upper bound on Template Revision references returned for one archive.
const MAX_REFERENCES: usize = 200;

impl Store {
    pub async fn artifact_metadata(
        &self,
        digest: &str,
    ) -> StoreResult<Option<TemplateArtifactMetadata>> {
        let db = self.connection();
        let Some((size_bytes, created_at)) = archives::Entity::find_by_id(digest)
            .select_only()
            .expr(Func::cust(Alias::new("length")).arg(col(archives::Column::Archive)))
            .column(archives::Column::CreatedAt)
            .into_tuple::<(i64, i64)>()
            .one(db)
            .await?
        else {
            return Ok(None);
        };
        let source_keys = sources::Entity::find()
            .select_only()
            .column(sources::Column::Key)
            .filter(sources::Column::ArtifactDigest.eq(digest))
            .order_by_asc(sources::Column::Key)
            .into_tuple::<String>()
            .all(db)
            .await?;
        let mut references = revisions::Entity::find()
            .select_only()
            .columns([
                revisions::Column::ProfileKey,
                revisions::Column::Revision,
                revisions::Column::State,
            ])
            .filter(revisions::Column::ArtifactDigest.eq(digest))
            .order_by_asc(revisions::Column::ProfileKey)
            .order_by_asc(revisions::Column::Revision)
            .limit(MAX_REFERENCES as u64 + 1)
            .into_tuple::<(String, i64, String)>()
            .all(db)
            .await?
            .into_iter()
            .map(|(profile_key, revision, state)| TemplateArtifactReference {
                profile_key,
                revision,
                state,
            })
            .collect::<Vec<_>>();
        let revisions_truncated = references.len() > MAX_REFERENCES;
        references.truncate(MAX_REFERENCES);
        Ok(Some(TemplateArtifactMetadata {
            digest: digest.to_owned(),
            size_bytes,
            created_at,
            source_keys,
            revisions: references,
            revisions_truncated,
        }))
    }
}
