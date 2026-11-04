use crate::StoreResult;
use crate::domain::customer_emails::{DEFAULT_REMINDER_SEND_HOUR, EmailKind};
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::invoicing_entities::{FooterTranslation, FooterTranslations, InvoicingEntity};
use crate::domain::{
    InvoicingEntityNew, InvoicingEntityPatch, InvoicingEntityProviders,
    InvoicingEntityProvidersPatch, TaxResolverEnum,
};
use crate::errors::StoreError;
use crate::repositories::connectors::ConnectorsInterface;
use crate::services::customer_locale::customer_default_locale;
use crate::services::utils::is_valid_number_pattern;
use crate::store::{PgConn, Store, StoreInternal};
use crate::utils::text::blank_to_none;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, InvoicingEntityId, OrganizationId, TaxCategoryId, TenantId};
use uuid::uuid;

/// Built-in "Digital services" tax category (seeded, standard-rated) — the default
/// classification for a new invoicing entity on this SaaS-first platform. Keep in
/// sync with the tax_categories migration seed.
const DIGITAL_SERVICES_TAX_CATEGORY_ID: TaxCategoryId =
    TaxCategoryId::from_const(uuid!("a0000000-0000-4000-8000-000000000004"));
use diesel_models::invoicing_entities::{
    InvoicingEntityProvidersRow, InvoicingEntityRow, InvoicingEntityRowPatch,
    InvoicingEntityRowProvidersPatch,
};
use diesel_models::invoicing_entity_email_settings::InvoicingEntityEmailSettingsRowNew;
use diesel_models::organizations::OrganizationRow;
use diesel_models::tax_categories::TaxCategoryRow;
use diesel_models::tenants::TenantRow;
use error_stack::Report;
use meteroid_store_macros::with_conn_delegate;
use scoped_futures::ScopedFutureExt;

#[with_conn_delegate]
#[async_trait::async_trait]
pub trait InvoicingEntityInterface {
    async fn list_invoicing_entities(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<InvoicingEntity>>;

    async fn list_invoicing_entities_by_ids(
        &self,
        ids: Vec<InvoicingEntityId>,
    ) -> StoreResult<Vec<InvoicingEntity>>;

    #[delegated]
    async fn get_invoicing_entity(
        &self,
        tenant_id: TenantId,
        invoicing_id_or_default: Option<InvoicingEntityId>,
    ) -> StoreResult<InvoicingEntity>;

    async fn create_invoicing_entity(
        &self,
        invoicing_entity: InvoicingEntityNew,
        tenant_id: TenantId,
        organization_id: OrganizationId,
    ) -> StoreResult<InvoicingEntity>;
    async fn patch_invoicing_entity(
        &self,
        actor: Actor,
        invoicing_entity: InvoicingEntityPatch,
        tenant_id: TenantId,
    ) -> StoreResult<InvoicingEntity>;

    async fn patch_invoicing_entity_providers(
        &self,
        actor: Actor,
        invoicing_entity: InvoicingEntityProvidersPatch,
        tenant_id: TenantId,
    ) -> StoreResult<InvoicingEntityProviders>;

    async fn resolve_providers_by_id(
        &self,
        tenant_id: TenantId,
        id: InvoicingEntityId,
    ) -> StoreResult<InvoicingEntityProviders>;
}

#[async_trait::async_trait]
impl InvoicingEntityInterface for Store {
    async fn list_invoicing_entities(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<InvoicingEntity>> {
        let mut conn = self.get_conn().await?;

        let invoicing_entities = InvoicingEntityRow::list_by_tenant_id(&mut conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::Into::into)
            .collect();

        Ok(invoicing_entities)
    }

    async fn list_invoicing_entities_by_ids(
        &self,
        ids: Vec<InvoicingEntityId>,
    ) -> StoreResult<Vec<InvoicingEntity>> {
        let mut conn = self.get_conn().await?;

        let invoicing_entities = InvoicingEntityRow::list_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::Into::into)
            .collect();

        Ok(invoicing_entities)
    }

    async fn get_invoicing_entity_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_id_or_default: Option<InvoicingEntityId>,
    ) -> StoreResult<InvoicingEntity> {
        let invoicing_entity = match invoicing_id_or_default {
            Some(invoicing_id) => InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                conn,
                invoicing_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into(),
            None => InvoicingEntityRow::get_default_invoicing_entity_for_tenant(conn, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into(),
        };

        Ok(invoicing_entity)
    }

    async fn create_invoicing_entity(
        &self,
        invoicing_entity: InvoicingEntityNew,
        tenant_id: TenantId,
        organization_id: OrganizationId,
    ) -> StoreResult<InvoicingEntity> {
        let mut conn = self.get_conn().await?;

        let organization = OrganizationRow::get_by_id(&mut conn, organization_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.internal
            .create_invoicing_entity(
                &mut conn,
                invoicing_entity,
                tenant_id,
                organization.default_country,
                organization.trade_name,
            )
            .await
    }

    async fn patch_invoicing_entity(
        &self,
        actor: Actor,
        invoicing_entity: InvoicingEntityPatch,
        tenant_id: TenantId,
    ) -> StoreResult<InvoicingEntity> {
        let mut conn = self.get_conn().await?;

        let mut row: InvoicingEntityRowPatch = invoicing_entity.into();
        let entity_id = row.id;

        if let Some(pattern) = &row.credit_note_number_pattern {
            ensure_valid_credit_note_number_pattern(pattern)?;
        }

        if let Some(locale) = row.customer_locale.take() {
            row.customer_locale = Some(normalize_customer_locale(locale.as_deref())?);
        }

        if let Some(color) = row.brand_color_light.take() {
            row.brand_color_light = Some(normalize_brand_color_light(color.as_deref())?);
        }

        if let Some(Some(category_id)) = row.default_tax_category_id {
            let available =
                TaxCategoryRow::is_available_for_tenant(&mut conn, category_id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
            if !available {
                return Err(Report::new(StoreError::InvalidArgument(
                    "default_tax_category_id is not a tax category available to this tenant"
                        .to_string(),
                )));
            }
        }

        if row.country.is_some() {
            let is_in_use = InvoicingEntityRow::is_in_use(&mut conn, row.id, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            // we don't allow country changes if already in use
            if is_in_use {
                row.country = None;
            } else {
                let currency = self
                    .internal
                    .get_currency_from_country(&row.country.clone().unwrap())?;
                row.accounting_currency = Some(currency);
            }
        }

        // Extras drop the post-patch default, so that state is rebuilt from the current row.
        if let Some(extras) = row.customer_extra_locales.take() {
            let current = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                &mut conn, entity_id, tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
            let mut after = InvoicingEntity::from(current);
            if let Some(locale) = &row.customer_locale {
                after.customer_locale = locale.clone();
            }
            if let Some(language) = &row.invoicing_language {
                after.invoicing_language = language.clone();
            }
            if let Some(country) = &row.country {
                after.country = country.clone();
            }
            let extras: Vec<String> = extras.into_iter().flatten().collect();
            let normalized =
                normalize_customer_extra_locales(&extras, customer_default_locale(&after))?;
            row.customer_extra_locales = Some(normalized.into_iter().map(Some).collect());
        }

        // Validate the tax resolver against the EFFECTIVE post-patch state (a
        // country change applied in this same patch must be honoured): the built-in
        // EU VAT engine is EU-seller-only, and an external tax provider may only be
        // referenced under the External resolver. `row.country` is the applied new
        // country (nulled above if the entity is already in use), else unchanged.
        if let Some(new_resolver) = row.tax_resolver {
            let current = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                &mut conn, entity_id, tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            let effective_country = row
                .country
                .clone()
                .unwrap_or_else(|| current.country.clone());

            if matches!(
                new_resolver,
                diesel_models::enums::TaxResolverEnum::MeteroidEuVat
            ) && !world_tax::is_eu_seller_country(&effective_country.code)
            {
                return Err(Report::new(StoreError::InvalidArgument(
                    "Meteroid EU VAT is available to EU sellers only; use an external tax provider for non-EU sellers"
                        .to_string(),
                )));
            }

            if !matches!(
                new_resolver,
                diesel_models::enums::TaxResolverEnum::External
            ) && current.tax_provider_id.is_some()
            {
                return Err(Report::new(StoreError::InvalidArgument(
                    "tax_provider_id is only valid with the External tax resolver".to_string(),
                )));
            }
        }

        let res: InvoicingEntityRow = self
            .transaction_bumping_tenants(|conn| {
                let actor = &actor;
                async move {
                    let before = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                        conn, entity_id, tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let res = row
                        .patch_invoicing_entity(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let changed = invoicing_entity_changed_fields(&before, &res);
                    if let Some(activity) = changed_fields_activity(entity_id, changed) {
                        self.internal
                            .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                            .await?;
                    }

                    let bumped = changes_credit_inputs(&before, &res).then_some(tenant_id);
                    Ok((res, bumped.into_iter().collect()))
                }
                .scope_boxed()
            })
            .await?;

        Ok(res.into())
    }

    async fn patch_invoicing_entity_providers(
        &self,
        actor: Actor,
        invoicing_entity: InvoicingEntityProvidersPatch,
        tenant_id: TenantId,
    ) -> StoreResult<InvoicingEntityProviders> {
        let entity_id = invoicing_entity.id;

        // Enforce provider capabilities before persisting: a connector newly assigned to a slot
        // must be active and have that rail enabled on its provider account. Otherwise the
        // misconfiguration is invisible until a customer tries to pay and the charge/mandate
        // fails at the provider (e.g. GoCardless has no card support). A slot left unchanged is
        // not re-checked, so a disabled connector doesn't block saving the other slots.
        let current = {
            let mut conn = self.get_conn().await?;
            InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                &mut conn, entity_id, tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        };
        for (slot, existing, requires_cards, label) in [
            (
                invoicing_entity.card_provider_id,
                current.card_provider_id,
                true,
                "card payments",
            ),
            (
                invoicing_entity.direct_debit_provider_id,
                current.direct_debit_provider_id,
                false,
                "direct debit",
            ),
        ] {
            if let Some(Some(connector_id)) = slot
                && existing != Some(connector_id)
            {
                let connector = self
                    .get_connector_with_data(connector_id, tenant_id)
                    .await?;
                if connector.provider().as_payment().is_none() {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "{:?} is not a payment provider",
                        connector.provider()
                    ))));
                }
                if !connector.is_active() {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "'{}' isn't active and can't be assigned to {label}",
                        connector.alias
                    ))));
                }

                // "Supports direct debit" means an actual DD rail: a card-only provider also
                // has supports_mandates (a saved card is a reusable mandate) but must never
                // land here.
                let supported = if requires_cards {
                    connector.supports_card()
                } else {
                    connector.supports_direct_debit()
                };

                if !supported {
                    return Err(Report::new(StoreError::InvalidArgument(format!(
                        "'{}' doesn't have {label} enabled on its provider account",
                        connector.alias
                    ))));
                }
            }
        }

        let row: InvoicingEntityRowProvidersPatch = invoicing_entity.into();

        let res = self
            .transaction(|conn| {
                let actor = &actor;
                async move {
                    let before = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                        conn, entity_id, tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                    let patched = row
                        .patch_invoicing_entity_providers(conn, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;

                    let changed = provider_changed_fields(&before, &patched);
                    if let Some(activity) = changed_fields_activity(entity_id, changed) {
                        self.internal
                            .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                            .await?;
                    }

                    InvoicingEntityProvidersRow::resolve_providers_by_id(
                        conn, patched.id, tenant_id,
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                }
                .scope_boxed()
            })
            .await?;

        Ok(res.into())
    }

    async fn resolve_providers_by_id(
        &self,
        tenant_id: TenantId,
        id: InvoicingEntityId,
    ) -> StoreResult<InvoicingEntityProviders> {
        let mut conn = self.get_conn().await?;

        let res = InvoicingEntityProvidersRow::resolve_providers_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(res.into())
    }
}

/// Builds the `entity.updated` audit activity for an invoicing entity from a set of
/// changed field names, or `None` when nothing changed (so no empty row is recorded).
fn changed_fields_activity(
    entity_id: InvoicingEntityId,
    changed: Vec<&'static str>,
) -> Option<Activity> {
    if changed.is_empty() {
        return None;
    }
    let activity = Activity::new(
        ActivityType::EntityUpdated,
        EntityType::InvoicingEntity,
        entity_id.as_uuid(),
    )
    .with_metadata(serde_json::json!({
        "changes": changed
            .iter()
            .map(|f| serde_json::json!({ "field": f }))
            .collect::<Vec<_>>(),
    }));
    Some(activity)
}

fn invalid_argument(msg: String) -> Report<StoreError> {
    Report::new(StoreError::InvalidArgument(msg))
}

/// Blank clears (follow the invoice language); otherwise a supported tag, stored normalized.
fn normalize_customer_locale(tag: Option<&str>) -> StoreResult<Option<String>> {
    match tag.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(None),
        Some(t) if common_domain::lang::is_supported(t) => {
            Ok(Some(common_domain::lang::template_lang(t).to_string()))
        }
        Some(t) => Err(invalid_argument(format!(
            "Unsupported email locale {t:?}; supported: {}",
            common_domain::lang::SUPPORTED.join(", ")
        ))),
    }
}

/// Blank clears (emails derive it from `brand_color`); otherwise a `#rgb` / `#rrggbb` hex colour.
fn normalize_brand_color_light(color: Option<&str>) -> StoreResult<Option<String>> {
    match color.map(str::trim).filter(|c| !c.is_empty()) {
        None => Ok(None),
        Some(c)
            if c.strip_prefix('#').is_some_and(|hex| {
                matches!(hex.len(), 3 | 6) && hex.bytes().all(|b| b.is_ascii_hexdigit())
            }) =>
        {
            Ok(Some(c.to_string()))
        }
        Some(c) => Err(invalid_argument(format!(
            "brand_color_light must be a #RRGGBB or #RGB hex colour, got {c:?}"
        ))),
    }
}

/// Supported template locales only, normalized (`de-AT` -> `de-DE`), deduped, default dropped.
fn normalize_customer_extra_locales(
    extras: &[String],
    default_locale: &str,
) -> StoreResult<Vec<String>> {
    let supported = common_domain::lang::SUPPORTED;
    if extras.len() > supported.len() {
        return Err(invalid_argument(format!(
            "extra_locales can hold at most {} locales",
            supported.len()
        )));
    }
    let mut normalized: Vec<String> = Vec::new();
    for raw in extras {
        let tag = raw.trim();
        if tag.is_empty() {
            continue;
        }
        if !common_domain::lang::is_supported(tag) {
            return Err(invalid_argument(format!(
                "Unsupported email locale {tag:?}; supported: {}",
                supported.join(", ")
            )));
        }
        let locale = common_domain::lang::template_lang(tag);
        if locale != default_locale && !normalized.iter().any(|l| l == locale) {
            normalized.push(locale.to_string());
        }
    }
    Ok(normalized)
}

fn ensure_valid_credit_note_number_pattern(pattern: &str) -> StoreResult<()> {
    if is_valid_number_pattern(pattern) {
        Ok(())
    } else {
        Err(Report::new(StoreError::InvalidArgument(
            "credit_note_number_pattern must contain a {number} or {number:N} placeholder"
                .to_string(),
        )))
    }
}

/// Value-level diff over the caller-editable columns, so the audit row reflects what
/// actually changed rather than the whole tab's field set.
fn invoicing_entity_changed_fields(
    before: &InvoicingEntityRow,
    after: &InvoicingEntityRow,
) -> Vec<&'static str> {
    let mut changed = Vec::new();
    macro_rules! diff {
        ($field:ident) => {
            if before.$field != after.$field {
                changed.push(stringify!($field));
            }
        };
    }
    diff!(legal_name);
    diff!(invoice_number_pattern);
    diff!(credit_note_number_pattern);
    diff!(grace_period_hours);
    diff!(net_terms);
    diff!(invoice_footer_info);
    diff!(invoice_footer_legal);
    diff!(logo_attachment_id);
    diff!(brand_color);
    diff!(address_line1);
    diff!(address_line2);
    diff!(zip_code);
    diff!(state);
    diff!(city);
    diff!(vat_number);
    diff!(country);
    diff!(tax_resolver);
    diff!(consolidate_recurring_invoices);
    diff!(portal_theme_mode);
    diff!(portal_roundness);
    diff!(require_vies_valid_for_reverse_charge);
    diff!(invoicing_language);
    diff!(customer_locale);
    diff!(customer_extra_locales);
    diff!(brand_color_light);
    diff!(portal_credit_top_up_enabled);
    diff!(portal_auto_top_up_enabled);
    diff!(portal_budgets_enabled);
    diff!(require_billing_information);
    changed
}

fn provider_changed_fields(
    before: &InvoicingEntityRow,
    after: &InvoicingEntityRow,
) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if before.card_provider_id != after.card_provider_id {
        changed.push("card_provider");
    }
    if before.direct_debit_provider_id != after.direct_debit_provider_id {
        changed.push("direct_debit_provider");
    }
    if before.bank_account_id != after.bank_account_id {
        changed.push("bank_account");
    }
    changed
}

impl StoreInternal {
    pub async fn create_invoicing_entity(
        &self,
        conn: &mut PgConn,
        invoicing_entity: InvoicingEntityNew,
        tenant_id: TenantId,
        default_country: CountryCode,
        trade_name: String,
    ) -> StoreResult<InvoicingEntity> {
        let other_exists = InvoicingEntityRow::exists_any_for_tenant(conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let country = invoicing_entity
            .country
            .clone()
            .unwrap_or(default_country.clone());

        let currency = self.get_currency_from_country(&country)?;

        if let Some(pattern) = &invoicing_entity.credit_note_number_pattern {
            ensure_valid_credit_note_number_pattern(pattern)?;
        }

        // Seed France's mandatory late-payment mentions into the (editable) footer on
        // create, so a new French entity is compliant by default without any
        // auto-rendering — the seller can edit or remove them like any footer text.
        // Base is French (the seller's home language, used for any unmatched locale);
        // an en-US translation covers English invoices.
        const FR_STATUTORY_FOOTER_LEGAL_FR: &str = "Retard de paiement : pénalités au taux de refinancement de la BCE majoré de 10 points, exigibles de plein droit.\nIndemnité forfaitaire pour frais de recouvrement : 40 € (art. L441-10 du Code de commerce).";
        const FR_STATUTORY_FOOTER_LEGAL_EN: &str = "Late payment: penalties at the ECB refinancing rate plus 10 points, due by operation of law.\nFixed recovery-cost indemnity: €40 (art. L441-10, French Commercial Code).";

        let seed_fr_statutory = country.code == "FR"
            && invoicing_entity
                .invoice_footer_legal
                .as_deref()
                .is_none_or(|s| s.trim().is_empty());

        let invoice_footer_legal = if seed_fr_statutory {
            Some(FR_STATUTORY_FOOTER_LEGAL_FR.to_string())
        } else {
            invoicing_entity.invoice_footer_legal.clone()
        };

        let invoice_footer_translations = if seed_fr_statutory {
            let mut by_locale = std::collections::BTreeMap::new();
            by_locale.insert(
                "en-US".to_string(),
                FooterTranslation {
                    info: None,
                    legal: Some(FR_STATUTORY_FOOTER_LEGAL_EN.to_string()),
                },
            );
            FooterTranslations(by_locale)
        } else {
            invoicing_entity
                .invoice_footer_translations
                .clone()
                .unwrap_or_default()
        };

        let customer_locale =
            normalize_customer_locale(invoicing_entity.customer_locale.as_deref())?;
        let customer_extra_locales = invoicing_entity.customer_extra_locales.clone();
        let brand_color_light =
            normalize_brand_color_light(invoicing_entity.brand_color_light.as_deref())?;

        let mut entity = InvoicingEntity {
            id: InvoicingEntityId::new(),
            einvoicing_format: None,
            legal_number: None,
            contact_name: None,
            contact_phone: None,
            contact_email: None,
            is_default: !other_exists,
            legal_name: invoicing_entity.legal_name.unwrap_or(trade_name),
            invoice_number_pattern: invoicing_entity
                .invoice_number_pattern
                .unwrap_or("INV-{number}".to_string()),
            credit_note_number_pattern: invoicing_entity
                .credit_note_number_pattern
                .unwrap_or("CN-{number:6}".to_string()),
            next_invoice_number: 1,
            next_credit_note_number: 1,
            grace_period_hours: invoicing_entity.grace_period_hours.unwrap_or(24),
            consolidate_recurring_invoices: false,
            net_terms: invoicing_entity.net_terms.unwrap_or(30),
            invoice_footer_info: invoicing_entity.invoice_footer_info.clone(),
            invoice_footer_legal,
            logo_attachment_id: invoicing_entity.logo_attachment_id,
            brand_color: invoicing_entity.brand_color.clone(),
            address_line1: invoicing_entity.address_line1.clone(),
            address_line2: invoicing_entity.address_line2.clone(),
            zip_code: invoicing_entity.zip_code.clone(),
            state: invoicing_entity.state.clone(),
            city: invoicing_entity.city.clone(),
            vat_number: invoicing_entity.vat_number.clone(),
            country,
            accounting_currency: currency.clone(),
            tenant_id,
            card_provider_id: None,
            direct_debit_provider_id: None,
            bank_account_id: None,
            tax_resolver: TaxResolverEnum::Manual,
            portal_theme_mode: invoicing_entity.portal_theme_mode.clone(),
            portal_roundness: invoicing_entity.portal_roundness.clone(),
            require_vies_valid_for_reverse_charge: invoicing_entity
                .require_vies_valid_for_reverse_charge
                .unwrap_or(false),
            invoicing_language: blank_to_none(invoicing_entity.invoicing_language.clone()),
            invoice_footer_translations,
            default_tax_category_id: Some(DIGITAL_SERVICES_TAX_CATEGORY_ID),
            tax_provider_id: None,
            customer_locale,
            customer_extra_locales: vec![],
            brand_color_light,
            portal_credit_top_up_enabled: invoicing_entity
                .portal_credit_top_up_enabled
                .unwrap_or(false),
            portal_auto_top_up_enabled: invoicing_entity
                .portal_auto_top_up_enabled
                .unwrap_or(false),
            portal_budgets_enabled: invoicing_entity.portal_budgets_enabled.unwrap_or(false),
            require_billing_information: invoicing_entity
                .require_billing_information
                .unwrap_or(false),
        };
        if let Some(extras) = customer_extra_locales {
            entity.customer_extra_locales =
                normalize_customer_extra_locales(&extras, customer_default_locale(&entity))?;
        }

        let row: InvoicingEntityRow = entity.into();

        let invoicing_entity_row = row
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Add the currency to tenant's available_currencies if not already present
        TenantRow::add_currency_if_missing(conn, tenant_id, currency)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // New email kinds stay off until an operator enables them (D22); the default reminder
        // sequence is seeded disabled (D30).
        InvoicingEntityEmailSettingsRowNew {
            invoicing_entity_id: invoicing_entity_row.id,
            tenant_id,
            sender_name: None,
            reply_to: None,
            bcc: vec![],
            disabled_kinds: EmailKind::default_disabled()
                .iter()
                .map(|k| Some(k.to_string()))
                .collect(),
            timezone: None,
            reminder_send_hour: DEFAULT_REMINDER_SEND_HOUR as i16,
        }
        .insert_if_absent(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        self.ensure_default_sequence(conn, tenant_id, invoicing_entity_row.id)
            .await?;

        Ok(invoicing_entity_row.into())
    }
}

/// Tax, currency and draft timing (grace, consolidation) feed live credit; numbering does not.
fn changes_credit_inputs(before: &InvoicingEntityRow, after: &InvoicingEntityRow) -> bool {
    before.country != after.country
        || before.accounting_currency != after.accounting_currency
        || before.vat_number != after.vat_number
        || before.tax_resolver != after.tax_resolver
        || before.require_vies_valid_for_reverse_charge
            != after.require_vies_valid_for_reverse_charge
        || before.default_tax_category_id != after.default_tax_category_id
        || before.tax_provider_id != after.tax_provider_id
        || before.grace_period_hours != after.grace_period_hours
        || before.consolidate_recurring_invoices != after.consolidate_recurring_invoices
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn customer_locale_is_normalized_and_blank_clears() {
        assert_eq!(
            normalize_customer_locale(Some(" de-AT ")).unwrap(),
            Some("de-DE".to_string())
        );
        assert_eq!(normalize_customer_locale(Some("  ")).unwrap(), None);
        assert_eq!(normalize_customer_locale(None).unwrap(), None);
        let err = normalize_customer_locale(Some("pl-PL")).unwrap_err();
        assert!(
            err.current_context().to_string().contains("pl-PL"),
            "{err:?}"
        );
    }

    #[test]
    fn brand_color_light_is_hex_and_blank_clears() {
        assert_eq!(
            normalize_brand_color_light(Some(" #1A2B3C ")).unwrap(),
            Some("#1A2B3C".to_string())
        );
        assert_eq!(
            normalize_brand_color_light(Some("#abc")).unwrap(),
            Some("#abc".to_string())
        );
        assert_eq!(normalize_brand_color_light(Some("  ")).unwrap(), None);
        assert_eq!(normalize_brand_color_light(None).unwrap(), None);
        for bad in ["red", "#12345", "1a2b3c", "#fff;color:red"] {
            assert!(normalize_brand_color_light(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn customer_extra_locales_are_normalized_deduped_and_drop_the_default() {
        let normalized = normalize_customer_extra_locales(
            &strings(&["de-DE", " de-AT ", "fr-FR", "", "pt-BR", "es"]),
            "fr-FR",
        )
        .unwrap();
        assert_eq!(normalized, vec!["de-DE", "pt-BR", "es-ES"]);

        let err =
            normalize_customer_extra_locales(&strings(&["de-DE", "pl-PL"]), "fr-FR").unwrap_err();
        assert!(
            err.current_context().to_string().contains("pl-PL"),
            "{err:?}"
        );
        assert!(normalize_customer_extra_locales(&strings(&["de-DE"; 9]), "fr-FR").is_err());
        assert!(
            normalize_customer_extra_locales(&[], "fr-FR")
                .unwrap()
                .is_empty()
        );
    }
}
