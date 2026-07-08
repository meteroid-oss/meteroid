use common_domain::ids::{StoredDocumentId, TenantId};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use meteroid_store::{Store, StoreResult, domain::InvoicingEntity};

/// Portal theme defaults configured on an invoicing entity, with the logo
/// attachment resolved through the same inheritance so callers can build the URL.
pub struct PortalBranding {
    pub brand_color: Option<String>,
    pub theme_mode: Option<String>,
    pub roundness: Option<String>,
    pub logo_attachment_id: Option<StoredDocumentId>,
}

/// Resolve portal theme/branding for an invoicing entity, inheriting any unset
/// value from the tenant's default entity. Mirrors the customer-overview logic
/// so standalone checkout / invoice-payment pages theme identically to the portal.
pub async fn resolve_portal_branding(
    store: &Store,
    tenant: TenantId,
    entity: &InvoicingEntity,
) -> StoreResult<PortalBranding> {
    let default_entity = if entity.is_default {
        None
    } else {
        Some(store.get_invoicing_entity(tenant, None).await?)
    };

    let derive = |own: Option<String>, pick: &dyn Fn(&InvoicingEntity) -> Option<String>| {
        own.or_else(|| default_entity.as_ref().and_then(pick))
    };

    Ok(PortalBranding {
        brand_color: derive(entity.brand_color.clone(), &|e| e.brand_color.clone()),
        theme_mode: derive(entity.portal_theme_mode.clone(), &|e| {
            e.portal_theme_mode.clone()
        }),
        roundness: derive(entity.portal_roundness.clone(), &|e| {
            e.portal_roundness.clone()
        }),
        logo_attachment_id: entity
            .logo_attachment_id
            .or_else(|| default_entity.as_ref().and_then(|e| e.logo_attachment_id)),
    })
}
