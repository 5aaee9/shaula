//! Keyset pages over registry keys for the management list endpoints.

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use shaula_core::registry::KeyPage;

use crate::entities::fleet::fleets;
use crate::entities::template::template_profiles;
use crate::store::{Store, StoreResult};

impl Store {
    /// Active (non-tombstoned) Fleets ordered by key, strictly after `page.after`.
    pub(crate) async fn fleet_page(&self, page: &KeyPage) -> StoreResult<Vec<fleets::Model>> {
        let mut query = fleets::Entity::find()
            .filter(fleets::Column::Tombstone.eq(false))
            .order_by_asc(fleets::Column::Key);
        if let Some(after) = &page.after {
            query = query.filter(fleets::Column::Key.gt(after.as_str()));
        }
        if let Some(limit) = page.limit {
            query = query.limit(limit as u64);
        }
        Ok(query.all(self.connection()).await?)
    }

    /// Template Profile keys ordered by key, strictly after `page.after`.
    pub(crate) async fn template_profile_key_page(
        &self,
        page: &KeyPage,
    ) -> StoreResult<Vec<String>> {
        let mut query = template_profiles::Entity::find()
            .select_only()
            .column(template_profiles::Column::Key)
            .order_by_asc(template_profiles::Column::Key);
        if let Some(after) = &page.after {
            query = query.filter(template_profiles::Column::Key.gt(after.as_str()));
        }
        if let Some(limit) = page.limit {
            query = query.limit(limit as u64);
        }
        Ok(query.into_tuple().all(self.connection()).await?)
    }
}
