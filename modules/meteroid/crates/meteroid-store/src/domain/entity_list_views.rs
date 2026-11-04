use chrono::NaiveDateTime;
use common_domain::ids::{EntityListViewId, TenantId, UserId};
use diesel_models::entity_list_views::EntityListViewRow;

use crate::domain::enums::EntityListType;

/// Longest accepted view name, in characters (mirrors the DB CHECK).
pub const LIST_VIEW_NAME_MAX_CHARS: usize = 100;
/// Largest accepted serialized `config`, in bytes.
pub const LIST_VIEW_CONFIG_MAX_BYTES: usize = 32 * 1024;
/// Most views a single user may own per list (private + shared).
pub const LIST_VIEWS_MAX_PER_USER: i64 = 50;

/// A saved list view as seen by a given actor.
#[derive(Clone, Debug)]
pub struct EntityListView {
    pub id: EntityListViewId,
    pub tenant_id: TenantId,
    pub entity_type: EntityListType,
    pub name: String,
    pub config: serde_json::Value,
    pub shared: bool,
    pub created_by: Option<UserId>,
    /// Display name of the creator, when known.
    pub created_by_name: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: Option<NaiveDateTime>,
}

impl EntityListView {
    pub(crate) fn from_row(row: EntityListViewRow, created_by_name: Option<String>) -> Self {
        EntityListView {
            id: row.id,
            tenant_id: row.tenant_id,
            entity_type: row.entity_type.into(),
            name: row.name,
            config: row.config,
            shared: row.shared,
            created_by: row.created_by,
            created_by_name,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    pub fn is_owned_by(&self, user: UserId) -> bool {
        self.created_by == Some(user)
    }
}

/// The views of one list visible to the actor, plus the actor's own default among them.
#[derive(Clone, Debug)]
pub struct EntityListViews {
    pub views: Vec<EntityListView>,
    pub default_view_id: Option<EntityListViewId>,
}

#[derive(Clone, Debug)]
pub struct EntityListViewNew {
    pub entity_type: EntityListType,
    pub name: String,
    pub config: serde_json::Value,
    pub shared: bool,
}

/// Partial update; `None` leaves a field untouched.
#[derive(Clone, Debug, Default)]
pub struct EntityListViewPatch {
    pub name: Option<String>,
    pub config: Option<serde_json::Value>,
    pub shared: Option<bool>,
}
