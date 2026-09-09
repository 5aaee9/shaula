//! Immutable archive bytes are authoritative; filesystem material is a cache.

use sea_orm::{ActiveValue::Set, EntityTrait, QueryOrder};
use sha2::{Digest, Sha256};
use shaula_core::registry::TemplateSource;

use crate::{Store, StoreError, StoreResult};

mod archives {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "artifact_archives")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub digest: String,
        pub archive: Vec<u8>,
        pub created_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

mod sources {
    use sea_orm::entity::prelude::*;
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "template_sources")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub key: String,
        pub artifact_digest: String,
        pub platform: String,
        pub engine_ref: String,
        pub created_at: i64,
    }
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}
}

impl Store {
    /// Saves original validated upload bytes once. Conflicting content is rejected.
    pub async fn artifact_archive_put(
        &self,
        digest: &str,
        bytes: &[u8],
        now: i64,
    ) -> StoreResult<()> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(StoreError::Corrupt(
                "artifact archive exceeds the size limit".into(),
            ));
        }
        if format!("sha256:{}", hex::encode(Sha256::digest(bytes))) != digest {
            return Err(StoreError::Corrupt(
                "artifact digest does not match archive".into(),
            ));
        }
        archives::Entity::insert(archives::ActiveModel {
            digest: Set(digest.to_owned()),
            archive: Set(bytes.to_vec()),
            created_at: Set(now),
        })
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(archives::Column::Digest)
                .do_nothing()
                .to_owned(),
        )
        .do_nothing()
        .exec(self.connection())
        .await?;
        if self.artifact_archive_get(digest).await?.as_deref() != Some(bytes) {
            return Err(StoreError::Corrupt(
                "stored artifact archive conflicts with digest".into(),
            ));
        }
        Ok(())
    }

    pub async fn artifact_archive_get(&self, digest: &str) -> StoreResult<Option<Vec<u8>>> {
        Ok(archives::Entity::find_by_id(digest.to_owned())
            .one(self.connection())
            .await?
            .map(|row| row.archive))
    }

    /// Lists identities only so restoring a library never loads every BLOB at once.
    pub async fn artifact_archive_digests(&self) -> StoreResult<Vec<String>> {
        use sea_orm::QuerySelect;
        let rows = archives::Entity::find()
            .select_only()
            .column(archives::Column::Digest)
            .order_by_asc(archives::Column::Digest)
            .into_tuple::<String>()
            .all(self.connection())
            .await?;
        Ok(rows)
    }

    pub async fn template_referenced_artifacts(&self) -> StoreResult<Vec<String>> {
        use crate::entities::template::template_profile_revisions as revisions;
        use sea_orm::QuerySelect;
        Ok(revisions::Entity::find()
            .select_only()
            .column(revisions::Column::ArtifactDigest)
            .distinct()
            .into_tuple::<String>()
            .all(self.connection())
            .await?)
    }

    /// Atomically replaces the discovery catalog with the current trusted sources.
    /// Archive bytes and published revisions remain available independently.
    pub async fn template_sources_replace(
        &self,
        current_sources: &[TemplateSource],
        now: i64,
    ) -> StoreResult<()> {
        let tx = self.begin().await?;
        sources::Entity::delete_many().exec(&tx).await?;
        for source in current_sources {
            sources::Entity::insert(sources::ActiveModel {
                key: Set(source.key.clone()),
                artifact_digest: Set(source.artifact_digest.clone()),
                platform: Set(source.platform.clone()),
                engine_ref: Set(source.engine_ref.clone()),
                created_at: Set(now),
            })
            .exec(&tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn template_sources(&self) -> StoreResult<Vec<TemplateSource>> {
        Ok(sources::Entity::find()
            .order_by_asc(sources::Column::Key)
            .all(self.connection())
            .await?
            .into_iter()
            .map(|row| TemplateSource {
                key: row.key,
                artifact_digest: row.artifact_digest,
                platform: row.platform,
                engine_ref: row.engine_ref,
            })
            .collect())
    }

    pub async fn template_source_get(&self, key: &str) -> StoreResult<Option<TemplateSource>> {
        Ok(sources::Entity::find_by_id(key.to_owned())
            .one(self.connection())
            .await?
            .map(|row| TemplateSource {
                key: row.key,
                artifact_digest: row.artifact_digest,
                platform: row.platform,
                engine_ref: row.engine_ref,
            }))
    }
}

#[cfg(test)]
#[path = "artifact_library_tests.rs"]
mod tests;
