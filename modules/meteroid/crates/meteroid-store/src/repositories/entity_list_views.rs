//! Saved list views: private-by-default, optionally shared with the tenant.
//!
//! Rules (enforced here and, for visibility, in SQL):
//! - A caller sees the tenant's shared views and their own private ones. Anything else is
//!   reported as not found.
//! - Any caller who can see a view may rename it, update its config or delete it. Only its
//!   creator may change whether it is shared.
//! - Each user has at most one default view per list, chosen among the views they can see.

use std::collections::HashMap;

use crate::domain::entity_list_views::{
    EntityListView, EntityListViewNew, EntityListViewPatch, EntityListViews,
    LIST_VIEW_CONFIG_MAX_BYTES, LIST_VIEW_NAME_MAX_CHARS, LIST_VIEWS_MAX_PER_USER,
};
use crate::domain::enums::EntityListType;
use crate::errors::StoreError;
use crate::{Store, StoreResult};
use chrono::Utc;
use common_domain::ids::{BaseId, EntityListViewId, TenantId, UserId};
use diesel_models::PgConn;
use diesel_models::entity_list_views::{
    EntityListViewDefaultRow, EntityListViewRow, EntityListViewRowNew, EntityListViewRowPatch,
};
use diesel_models::users::UserRow;
use error_stack::{Report, bail};
use scoped_futures::ScopedFutureExt;

#[async_trait::async_trait]
pub trait EntityListViewsInterface {
    async fn list_views(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        entity_type: EntityListType,
    ) -> StoreResult<EntityListViews>;

    async fn create_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        view: EntityListViewNew,
    ) -> StoreResult<EntityListView>;

    async fn update_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        id: EntityListViewId,
        patch: EntityListViewPatch,
    ) -> StoreResult<EntityListView>;

    async fn delete_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        id: EntityListViewId,
    ) -> StoreResult<()>;

    /// Copies a visible view into a new private view owned by `actor`.
    async fn duplicate_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        id: EntityListViewId,
        name: Option<String>,
    ) -> StoreResult<EntityListView>;

    /// Sets (or with `None`, clears) `actor`'s default view for a list.
    async fn set_default_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        entity_type: EntityListType,
        view_id: Option<EntityListViewId>,
    ) -> StoreResult<Option<EntityListViewId>>;
}

fn normalize_name(name: &str) -> StoreResult<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!(StoreError::InvalidArgument(
            "View name must not be empty".to_string()
        ));
    }
    if name.chars().count() > LIST_VIEW_NAME_MAX_CHARS {
        bail!(StoreError::InvalidArgument(format!(
            "View name must be at most {LIST_VIEW_NAME_MAX_CHARS} characters"
        )));
    }
    Ok(name.to_string())
}

fn validate_config(config: &serde_json::Value) -> StoreResult<()> {
    if !config.is_object() {
        bail!(StoreError::InvalidArgument(
            "View config must be a JSON object".to_string()
        ));
    }
    if config.to_string().len() > LIST_VIEW_CONFIG_MAX_BYTES {
        bail!(StoreError::InvalidArgument(format!(
            "View config must be at most {LIST_VIEW_CONFIG_MAX_BYTES} bytes"
        )));
    }
    Ok(())
}

fn not_found() -> Report<StoreError> {
    Report::new(StoreError::ValueNotFound("list view not found".to_string()))
}

/// Maps a row-level `NotFound` to the view-specific error, passing other errors through.
fn db_err(err: diesel_models::errors::DatabaseErrorContainer) -> Report<StoreError> {
    let report: Report<StoreError> = err.into();
    if matches!(report.current_context(), StoreError::ValueNotFound(_)) {
        not_found()
    } else {
        report
    }
}

fn display_name(user: &UserRow) -> String {
    let full = [user.first_name.as_deref(), user.last_name.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if full.is_empty() {
        user.email.clone()
    } else {
        full
    }
}

async fn with_creator_names(
    conn: &mut PgConn,
    rows: Vec<EntityListViewRow>,
) -> StoreResult<Vec<EntityListView>> {
    let mut ids: Vec<UserId> = rows.iter().filter_map(|r| r.created_by).collect();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup();

    let names: HashMap<UserId, String> = UserRow::find_by_ids(conn, &ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .iter()
        .map(|u| (u.id, display_name(u)))
        .collect();

    Ok(rows
        .into_iter()
        .map(|row| {
            let name = row.created_by.and_then(|id| names.get(&id).cloned());
            EntityListView::from_row(row, name)
        })
        .collect())
}

async fn one_with_creator_name(
    conn: &mut PgConn,
    row: EntityListViewRow,
) -> StoreResult<EntityListView> {
    with_creator_names(conn, vec![row])
        .await?
        .pop()
        .ok_or_else(not_found)
}

async fn ensure_quota(
    conn: &mut PgConn,
    tenant_id: TenantId,
    actor: UserId,
    entity_type: EntityListType,
) -> StoreResult<()> {
    let owned = EntityListViewRow::count_owned(conn, tenant_id, actor, entity_type.into())
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if owned >= LIST_VIEWS_MAX_PER_USER {
        bail!(StoreError::InvalidArgument(format!(
            "You can have at most {LIST_VIEWS_MAX_PER_USER} views on this list; delete one first"
        )));
    }
    Ok(())
}

#[async_trait::async_trait]
impl EntityListViewsInterface for Store {
    async fn list_views(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        entity_type: EntityListType,
    ) -> StoreResult<EntityListViews> {
        let mut conn = self.get_conn().await?;

        let rows = EntityListViewRow::list_visible(&mut conn, tenant_id, actor, entity_type.into())
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let default_view_id =
            EntityListViewDefaultRow::get_visible(&mut conn, tenant_id, actor, entity_type.into())
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        Ok(EntityListViews {
            views: with_creator_names(&mut conn, rows).await?,
            default_view_id,
        })
    }

    async fn create_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        view: EntityListViewNew,
    ) -> StoreResult<EntityListView> {
        let name = normalize_name(&view.name)?;
        validate_config(&view.config)?;

        let mut conn = self.get_conn().await?;
        ensure_quota(&mut conn, tenant_id, actor, view.entity_type).await?;

        let row = EntityListViewRowNew {
            id: EntityListViewId::new(),
            tenant_id,
            entity_type: view.entity_type.into(),
            name,
            config: view.config,
            shared: view.shared,
            created_by: Some(actor),
        }
        .insert(&mut conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        one_with_creator_name(&mut conn, row).await
    }

    async fn update_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        id: EntityListViewId,
        patch: EntityListViewPatch,
    ) -> StoreResult<EntityListView> {
        let name = patch.name.as_deref().map(normalize_name).transpose()?;
        if let Some(config) = &patch.config {
            validate_config(config)?;
        }

        let row = self
            .transaction(|conn| {
                async move {
                    let current =
                        EntityListViewRow::get_visible_for_update(conn, id, tenant_id, actor)
                            .await
                            .map_err(db_err)?;

                    let sharing_change = patch.shared.filter(|s| *s != current.shared);
                    if sharing_change.is_some() && current.created_by != Some(actor) {
                        bail!(StoreError::Forbidden(
                            "Only the creator of a view can change who it is shared with"
                                .to_string()
                        ));
                    }

                    let row = EntityListViewRow::patch_visible(
                        conn,
                        id,
                        tenant_id,
                        actor,
                        &EntityListViewRowPatch {
                            name,
                            config: patch.config,
                            shared: sharing_change,
                            updated_at: Some(Utc::now().naive_utc()),
                        },
                    )
                    .await
                    .map_err(db_err)?;

                    // Unsharing drops everyone else's default on it, so re-sharing later doesn't
                    // silently make it their default again.
                    if sharing_change == Some(false) {
                        EntityListViewDefaultRow::clear_for_view_except(conn, id, actor)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }

                    Ok(row)
                }
                .scope_boxed()
            })
            .await?;

        let mut conn = self.get_conn().await?;
        one_with_creator_name(&mut conn, row).await
    }

    async fn delete_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        id: EntityListViewId,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        // Anyone's default pointing at it goes with it (FK cascade).
        let deleted = EntityListViewRow::delete_visible(&mut conn, id, tenant_id, actor)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        if deleted == 0 {
            return Err(not_found());
        }
        Ok(())
    }

    async fn duplicate_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        id: EntityListViewId,
        name: Option<String>,
    ) -> StoreResult<EntityListView> {
        let mut conn = self.get_conn().await?;

        let original = EntityListViewRow::get_visible(&mut conn, id, tenant_id, actor)
            .await
            .map_err(db_err)?;

        let name = match name {
            Some(n) => normalize_name(&n)?,
            None => {
                // Keep the suffix within the length limit.
                let base: String = original
                    .name
                    .chars()
                    .take(LIST_VIEW_NAME_MAX_CHARS - " (copy)".len())
                    .collect();
                format!("{base} (copy)")
            }
        };

        let entity_type: EntityListType = original.entity_type.into();
        ensure_quota(&mut conn, tenant_id, actor, entity_type).await?;

        let row = EntityListViewRowNew {
            id: EntityListViewId::new(),
            tenant_id,
            entity_type: original.entity_type,
            name,
            config: original.config,
            shared: false,
            created_by: Some(actor),
        }
        .insert(&mut conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        one_with_creator_name(&mut conn, row).await
    }

    async fn set_default_view(
        &self,
        tenant_id: TenantId,
        actor: UserId,
        entity_type: EntityListType,
        view_id: Option<EntityListViewId>,
    ) -> StoreResult<Option<EntityListViewId>> {
        let mut conn = self.get_conn().await?;

        let Some(view_id) = view_id else {
            EntityListViewDefaultRow::clear(&mut conn, tenant_id, actor, entity_type.into())
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            return Ok(None);
        };

        let view = EntityListViewRow::get_visible(&mut conn, view_id, tenant_id, actor)
            .await
            .map_err(db_err)?;

        let view_entity: EntityListType = view.entity_type.into();
        if view_entity != entity_type {
            bail!(StoreError::InvalidArgument(
                "View belongs to a different list".to_string()
            ));
        }

        EntityListViewDefaultRow {
            tenant_id,
            user_id: actor,
            entity_type: entity_type.into(),
            view_id,
        }
        .upsert(&mut conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        Ok(Some(view_id))
    }
}
