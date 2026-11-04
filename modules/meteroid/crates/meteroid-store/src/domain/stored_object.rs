use super::enums::StoredObjectKindEnum;
use common_domain::ids::{StoredDocumentId, TenantId};
use diesel_models::stored_object::{StoredObjectRow, StoredObjectRowNew};
use o2o::o2o;

/// A freshly stored object: its minted id and the relative object-store key it was written under.
/// The storage service returns it, and callers persist the id (on the owning entity) together with
/// the key (in the registry).
#[derive(Debug, Clone)]
pub struct StoredObjectRef {
    pub id: StoredDocumentId,
    pub key: String,
}

/// Registry entry for one stored object: the tenant that owns it, its kind, and the relative
/// object-store key it lives at. The key is authoritative — reads `get` it directly and a tenant
/// wipe deletes it exactly, so no path layout has to be reconstructed.
#[derive(Debug, Clone, o2o)]
#[from_owned(StoredObjectRow)]
pub struct StoredObject {
    pub id: StoredDocumentId,
    pub tenant_id: TenantId,
    #[from(~.into())]
    pub kind: StoredObjectKindEnum,
    pub key: String,
}

/// A stored object to record in the registry.
#[derive(Debug, Clone, o2o)]
#[owned_into(StoredObjectRowNew)]
pub struct StoredObjectNew {
    pub id: StoredDocumentId,
    pub tenant_id: TenantId,
    #[into(~.into())]
    pub kind: StoredObjectKindEnum,
    pub key: String,
}
