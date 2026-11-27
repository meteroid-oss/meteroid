use diesel_async::RunQueryDsl;
use diesel_models::errors::DatabaseErrorContainer;
use diesel_models::query::tenant_copy::{CopiedTable, OF_TENANT, copy_tenant_rows};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::TenantEnvironmentEnum;
use crate::domain::purchase_links::generate_purchase_link_token;
use crate::domain::{Tenant, TenantNew};
use crate::errors::StoreError;
use crate::hooks::ModuleTenantCopy;
use crate::repositories::{OrganizationsInterface, TenantInterface};
use crate::store::{PgConn, Store};
use crate::{StoreResult, domain};
use common_domain::ids::{BaseId, OrganizationId, TenantId};

/// What a new tenant takes from an existing one. Customers, subscriptions, invoices,
/// payments and connector credentials are never copied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TenantCopyScope {
    /// Products, metrics, prices, plans and their versions, add-ons, coupons, features, purchase
    /// links, quote approval rules, and the modules' catalog configuration.
    pub catalog: bool,
    /// Invoicing entities, bank accounts, custom taxes, email and reminder settings.
    pub invoicing: bool,
}

/// Tokens are unique across tenants: each copy gets a fresh one once inserted.
const PLACEHOLDER_TOKEN: &str = "'copy:' || gen_random_uuid()";
const OF_PLAN_VERSIONS: &str =
    "s.plan_version_id IN (SELECT id FROM plan_version WHERE tenant_id = $1)";

const fn table(name: &'static str, source: &'static str) -> CopiedTable {
    CopiedTable::new(name, source)
}

const TAX_CATEGORIES: CopiedTable = CopiedTable {
    builtin: &[("parent_id", "tax_category")],
    ..table("tax_category", OF_TENANT)
};

const OF_ADD_ONS: &str = "s.add_on_id IN (SELECT id FROM add_on WHERE tenant_id = $1)";
const OF_COUPONS: &str = "s.coupon_id IN (SELECT id FROM coupon WHERE tenant_id = $1)";

const CATALOG: &[CopiedTable] = &[
    table("product_family", OF_TENANT),
    CopiedTable {
        mapped: &["parent_type_id"],
        ..table("resource_type", OF_TENANT)
    },
    CopiedTable {
        mapped: &["resource_type_id"],
        ..table("resource_type_key", OF_TENANT)
    },
    CopiedTable {
        mapped: &["product_family_id"],
        builtin: &[("tax_category_id", "tax_category")],
        ..table("product", OF_TENANT)
    },
    CopiedTable {
        mapped: &["product_family_id", "product_id", "attributed_to_type_id"],
        ..table("billable_metric", OF_TENANT)
    },
    table("managed_quantity", OF_TENANT),
    CopiedTable {
        mapped: &["product_id"],
        ..table("credit_unit", OF_TENANT)
    },
    CopiedTable {
        mapped: &["credit_unit_id"],
        ..table("credit_unit_price", OF_TENANT)
    },
    CopiedTable {
        mapped: &["metric_id", "credit_unit_id"],
        ..table("rate_card", OF_TENANT)
    },
    CopiedTable {
        mapped: &["rate_card_id"],
        ..table("rate_card_entry", OF_TENANT)
    },
    CopiedTable {
        mapped: &["product_id", "credit_unit_id", "supersedes"],
        ..table("price", OF_TENANT)
    },
    CopiedTable {
        mapped: &["product_family_id", "sold_for_resource_type_id"],
        mapped_arrays: &["pricing_product_order", "pricing_hidden_product_ids"],
        deferred: &["active_version_id", "draft_version_id"],
        ..table("plan", OF_TENANT)
    },
    CopiedTable {
        mapped: &["plan_id", "trialing_plan_id"],
        ..table("plan_version", OF_TENANT)
    },
    CopiedTable {
        mapped: &["plan_version_id"],
        ..table("plan_version_option", OF_TENANT)
    },
    CopiedTable {
        mapped: &["plan_version_id", "product_id", "billable_metric_id"],
        ..table("price_component", OF_PLAN_VERSIONS)
    },
    CopiedTable {
        mapped: &["plan_component_id", "price_id", "option_id"],
        ..table(
            "plan_component_price",
            "s.plan_component_id IN (SELECT pc.id FROM price_component pc \
             JOIN plan_version pv ON pv.id = pc.plan_version_id WHERE pv.tenant_id = $1)",
        )
    },
    CopiedTable {
        mapped: &["plan_version_id"],
        ..table("schedule", OF_PLAN_VERSIONS)
    },
    CopiedTable {
        mapped: &["plan_version_id"],
        ..table("plan_version_adjustment", OF_PLAN_VERSIONS)
    },
    CopiedTable {
        mapped: &["plan_version_id", "product_id"],
        ..table("plan_version_adjustment_component", OF_PLAN_VERSIONS)
    },
    CopiedTable {
        mapped: &["product_id", "price_id"],
        ..table("add_on", OF_TENANT)
    },
    CopiedTable {
        mapped: &["add_on_id", "price_id"],
        ..table("add_on_price", OF_ADD_ONS)
    },
    CopiedTable {
        mapped: &["plan_version_id", "add_on_id", "price_id"],
        ..table("plan_version_add_on", OF_TENANT)
    },
    CopiedTable {
        set: &[("redemption_count", "0"), ("last_redemption_at", "NULL")],
        ..table("coupon", OF_TENANT)
    },
    CopiedTable {
        mapped: &["coupon_id", "plan_id"],
        ..table("coupon_plan", OF_COUPONS)
    },
    CopiedTable {
        mapped: &["coupon_id", "add_on_id"],
        ..table("coupon_add_on", OF_COUPONS)
    },
    CopiedTable {
        mapped: &["product_id", "metric_id", "level_resource_type_id"],
        ..table("feature", OF_TENANT)
    },
    CopiedTable {
        mapped: &["feature_id", "managed_quantity_id"],
        ..table(
            "feature_quantity_limit",
            "s.feature_id IN (SELECT id FROM feature WHERE tenant_id = $1)",
        )
    },
    CopiedTable {
        mapped: &["feature_id", "entity_id", "level_type_id"],
        kept: &["created_by", "revoked_by"],
        cleared: &["resource_id"],
        regenerated: &["lineage_id"],
        ..table(
            "entitlement",
            "s.tenant_id = $1 AND s.entity_type IN ('FEATURE', 'PLAN', 'PLAN_VERSION', 'ADD_ON') \
             AND s.resource_id IS NULL",
        )
    },
    CopiedTable {
        mapped: &["product_family_id", "signup_plan_id"],
        ..table("product_family_journey", OF_TENANT)
    },
    CopiedTable {
        mapped: &["plan_id"],
        set: &[("token", PLACEHOLDER_TOKEN)],
        ..table("purchase_link", OF_TENANT)
    },
    CopiedTable {
        kept: &["approver_user_ids"],
        ..table("quote_approval_rule", OF_TENANT)
    },
    table("custom_property_definition", OF_TENANT),
    table("invoice_annex_template", OF_TENANT),
    CopiedTable {
        mapped: &["plan_version_id", "annex_template_id"],
        ..table("plan_version_annex_template", OF_TENANT)
    },
];

const INVOICING: &[CopiedTable] = &[
    table("bank_account", OF_TENANT),
    table("dunning_policy", OF_TENANT),
    CopiedTable {
        mapped: &["bank_account_id"],
        builtin: &[("default_tax_category_id", "tax_category")],
        cleared: &[
            "logo_attachment_id",
            "card_provider_id",
            "direct_debit_provider_id",
            "tax_provider_id",
        ],
        set: &[
            ("next_invoice_number", "1"),
            ("next_credit_note_number", "1"),
        ],
        ..table("invoicing_entity", OF_TENANT)
    },
    CopiedTable {
        mapped: &["invoicing_entity_id"],
        ..table("invoicing_entity_email_settings", OF_TENANT)
    },
    CopiedTable {
        mapped: &["invoicing_entity_id"],
        ..table("email_kind_audience", OF_TENANT)
    },
    CopiedTable {
        mapped: &["invoicing_entity_id"],
        ..table("payment_reminder_sequence", OF_TENANT)
    },
    CopiedTable {
        mapped: &["sequence_id"],
        ..table("payment_reminder_step", OF_TENANT)
    },
    CopiedTable {
        mapped: &["invoicing_entity_id"],
        builtin: &[("tax_category_id", "tax_category")],
        ..table("custom_tax", OF_TENANT)
    },
];

const OF_INVOICING_ENTITIES: &str =
    "s.invoicing_entity_id IN (SELECT id FROM invoicing_entity WHERE tenant_id = $1)";

const CATALOG_INVOICING: &[CopiedTable] = &[
    CopiedTable {
        mapped: &["product_id", "invoicing_entity_id"],
        ..table("product_accounting", OF_INVOICING_ENTITIES)
    },
    CopiedTable {
        mapped: &["product_id", "invoicing_entity_id", "custom_tax_id"],
        ..table("product_custom_tax", OF_INVOICING_ENTITIES)
    },
];

/// The tables of core a copy leaves behind, so that a new one has to be classified too.
pub const NOT_COPIED: &[&str] = &[
    // Customers and what they bought, were billed and paid.
    "applied_coupon",
    "billing_controller",
    "entitlement_app_count",
    "entitlement_app_count_op",
    "invoice_claims_moved",
    "invoice_dunning_cancel",
    "resource",
    "checkout_session",
    "credit_balance_projection",
    "credit_customer_inputs",
    "credit_input_change",
    "credit_note",
    "credit_recompute_mark",
    "credit_tenant_inputs",
    "credit_usage_settlement",
    "customer",
    "customer_balance_pending_tx",
    "customer_balance_tx",
    "customer_credit_account",
    "customer_credit_auto_top_up",
    "customer_credit_entry",
    "customer_credit_grant",
    "customer_credit_grant_product",
    "customer_credit_line_allocation",
    "customer_credit_operation",
    "customer_credit_purchase",
    "customer_credit_unrecovered",
    "customer_payment_method",
    "payment_method_expiry_notice",
    "invoice",
    "invoice_carry_forward",
    "invoice_payment_reminder",
    "payment_reversal_movement",
    "payment_transaction",
    "portal_token",
    "promo_code_attempt",
    "purchase_link_session",
    "quote",
    "quote_add_on",
    "quote_approval",
    "quote_comment",
    "quote_component",
    "quote_coupon",
    "quote_draft",
    "quote_evidence_event",
    "quote_recipient_engagement",
    "quote_revision",
    "quote_signature_certificate",
    "quote_signature_otp",
    "quote_view",
    "slot_transaction",
    "subscription",
    "subscription_add_on",
    "subscription_change",
    "subscription_component",
    "subscription_ending",
    "subscription_excluded_component",
    "subscription_plan_window",
    "subscription_settlement_due",
    "subscription_start_settled",
    "subscription_trial",
    "subscription_usage_filter",
    // Credentials and the integrations that hold them.
    "api_token",
    "connector",
    "connector_sync_issue",
    "webhook_out_endpoint",
    "webhook_out_retired_secret",
    // What the platform did for the tenant.
    "api_idempotency_key",
    "batch_job",
    "batch_job_chunk",
    "batch_job_entity",
    "batch_job_item_failure",
    "bi_mrr_movement_log",
    "dead_letter_message",
    "email_send_claim",
    "entity_activity",
    "outbox_event",
    "sandbox_seed",
    "sent_email",
    "stored_object",
    "webhook_out_delivery",
    "webhook_out_message",
    "webhook_out_message_version",
    // Analytics, set up again per tenant, and each member's own list views.
    "alert_definition",
    "entity_list_view",
    "entity_list_view_default",
];

/// Every table core copies, whatever the scope.
pub fn copied_by_core() -> impl Iterator<Item = &'static str> {
    std::iter::once(&TAX_CATEGORIES)
        .chain(CATALOG)
        .chain(INVOICING)
        .chain(CATALOG_INVOICING)
        .map(|t| t.table)
}

/// The rows `insert_tenant` creates that the copy replaces.
fn replaced_defaults(scope: TenantCopyScope) -> Vec<&'static str> {
    let mut tables = Vec::new();
    if scope.invoicing {
        tables.extend([
            "payment_reminder_step",
            "payment_reminder_sequence",
            "email_kind_audience",
            "invoicing_entity_email_settings",
            "invoicing_entity",
        ]);
    }
    if scope.catalog {
        tables.push("product_family");
    }
    tables
}

fn copied_tables(
    scope: TenantCopyScope,
    modules: &[ModuleTenantCopy],
) -> Vec<&'static CopiedTable> {
    let mut tables: Vec<&CopiedTable> = vec![&TAX_CATEGORIES];
    if scope.catalog {
        tables.extend(CATALOG);
    }
    if scope.invoicing {
        tables.extend(INVOICING);
    }
    if scope.catalog && scope.invoicing {
        tables.extend(CATALOG_INVOICING);
    }
    for module in modules {
        if scope.catalog {
            tables.extend(module.catalog);
        }
        if scope.invoicing {
            tables.extend(module.invoicing);
        }
    }
    tables
}

async fn regenerate_purchase_link_tokens(conn: &mut PgConn, target: TenantId) -> StoreResult<()> {
    #[derive(diesel::QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Uuid)]
        id: uuid::Uuid,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT id FROM purchase_link WHERE tenant_id = $1 AND token LIKE 'copy:%'",
    )
    .bind::<diesel::sql_types::Uuid, _>(target.as_uuid())
    .load(conn)
    .await
    .map_err(db_error)?;
    for row in rows {
        diesel::sql_query("UPDATE purchase_link SET token = $1 WHERE id = $2")
            .bind::<diesel::sql_types::Text, _>(generate_purchase_link_token())
            .bind::<diesel::sql_types::Uuid, _>(row.id)
            .execute(conn)
            .await
            .map_err(db_error)?;
    }
    Ok(())
}

#[derive(diesel::QueryableByName)]
struct Audience {
    #[diesel(sql_type = diesel::sql_types::Text)]
    rule: String,
}

/// Audience rules naming customers, which are never copied, or plans, when the catalog is
/// not: copied as is, they would silently reach someone else.
async fn refuse_audiences_of_absent_rows(
    conn: &mut PgConn,
    source: TenantId,
    scope: TenantCopyScope,
) -> StoreResult<()> {
    let rules: Vec<Audience> = diesel::sql_query(
        "WITH audiences AS ( \
           SELECT 'the \"' || replace(lower(kind), '_', ' ') || '\" emails' AS rule, audience FROM email_kind_audience \
           WHERE tenant_id = $1 \
           UNION ALL \
           SELECT 'the reminder sequence ' || name, audience FROM payment_reminder_sequence \
           WHERE tenant_id = $1 AND archived_at IS NULL AND audience IS NOT NULL) \
         SELECT rule FROM audiences \
         WHERE jsonb_path_exists(audience, '$.**.customer_ids[*]') \
            OR ($2 AND jsonb_path_exists(audience, '$.**.plan_ids[*]'))",
    )
    .bind::<diesel::sql_types::Uuid, _>(source.as_uuid())
    .bind::<diesel::sql_types::Bool, _>(!scope.catalog)
    .load(conn)
    .await
    .map_err(db_error)?;
    if rules.is_empty() {
        return Ok(());
    }
    let named = rules
        .into_iter()
        .map(|r| r.rule)
        .collect::<Vec<_>>()
        .join(", ");
    Err(StoreError::InvalidArgument(format!(
        "The audience of {named} names customers{}, which a copy cannot carry: edit it first",
        if scope.catalog { "" } else { " or plans" }
    ))
    .into())
}

fn db_error(e: diesel::result::Error) -> Report<StoreError> {
    DatabaseErrorContainer::from(e).into()
}

async fn copy_configuration(
    conn: &mut PgConn,
    source: TenantId,
    target: TenantId,
    scope: TenantCopyScope,
    modules: &[ModuleTenantCopy],
) -> StoreResult<Vec<(&'static str, usize)>> {
    if scope.invoicing {
        refuse_audiences_of_absent_rows(conn, source, scope).await?;
    }
    for table in replaced_defaults(scope) {
        diesel::sql_query(format!("DELETE FROM {table} WHERE tenant_id = $1"))
            .bind::<diesel::sql_types::Uuid, _>(target.as_uuid())
            .execute(conn)
            .await
            .map_err(db_error)?;
    }
    let copied = copy_tenant_rows(conn, source, target, &copied_tables(scope, modules))
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if scope.catalog {
        regenerate_purchase_link_tokens(conn, target).await?;
    }
    diesel::sql_query(
        "UPDATE tenant t SET reporting_currency = s.reporting_currency, \
           available_currencies = ARRAY(SELECT DISTINCT c FROM \
             unnest(t.available_currencies || s.available_currencies) c WHERE c IS NOT NULL ORDER BY c), \
           alert_evaluation_interval_secs = s.alert_evaluation_interval_secs, \
           email_test_recipients = s.email_test_recipients, \
           portal_embed_origins = s.portal_embed_origins \
         FROM tenant s WHERE s.id = $1 AND t.id = $2",
    )
    .bind::<diesel::sql_types::Uuid, _>(source.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(target.as_uuid())
    .execute(conn)
    .await
    .map_err(db_error)?;
    Ok(copied)
}

#[async_trait::async_trait]
pub trait TenantCopyInterface {
    /// A new tenant, set up like `source`: see [`TenantCopyScope`].
    async fn insert_tenant_copy(
        &self,
        actor: Actor,
        source: TenantId,
        tenant: TenantNew,
        organization_id: OrganizationId,
        scope: TenantCopyScope,
    ) -> StoreResult<TenantCopy>;
}

/// The new tenant, with the rows copied per table.
pub struct TenantCopy {
    pub tenant: Tenant,
    pub copied: Vec<(String, usize)>,
}

#[async_trait::async_trait]
impl TenantCopyInterface for Store {
    async fn insert_tenant_copy(
        &self,
        actor: Actor,
        source: TenantId,
        tenant: TenantNew,
        organization_id: OrganizationId,
        scope: TenantCopyScope,
    ) -> StoreResult<TenantCopy> {
        if tenant.environment == TenantEnvironmentEnum::Sandbox {
            return Err(StoreError::InvalidArgument(
                "A sandbox is seeded with its own data and cannot be a copy".to_string(),
            )
            .into());
        }
        let source_tenant = self.find_tenant_by_id(source).await?;
        if source_tenant.tenant.organization_id != organization_id {
            return Err(StoreError::ValueNotFound(format!("Tenant {source} not found")).into());
        }
        let organization = self
            .get_organizations_with_tenants_by_id(organization_id)
            .await?
            .organization;
        if tenant.environment == TenantEnvironmentEnum::Production {
            self.organization_hooks
                .ensure_production_tenant_allowed(organization_id)
                .await?;
        }
        let invoicing_entity = tenant.invoicing_entity.clone().unwrap_or_default();

        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let created = self
                    .internal
                    .insert_tenant_with_default_entities(
                        conn,
                        tenant,
                        organization_id,
                        organization.trade_name.clone(),
                        organization.default_country.clone(),
                        invoicing_entity,
                    )
                    .await?;
                let copied =
                    copy_configuration(conn, source, created.id, scope, &self.hooks.tenant_copy)
                        .await?;
                let activity = Activity::new(
                    ActivityType::EntityUpdated,
                    EntityType::Tenant,
                    created.id.as_uuid(),
                )
                .with_metadata(serde_json::json!({
                    "changes": [{ "field": "configuration" }],
                    "copied_from": source.as_base62(),
                    "catalog": scope.catalog,
                    "invoicing": scope.invoicing,
                }));
                self.internal
                    .record_audit_tx(conn, created.id, actor, AuditInput::Activity(activity))
                    .await?;
                let row = diesel_models::tenants::TenantRow::find_by_id(conn, created.id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                Ok::<TenantCopy, Report<StoreError>>(TenantCopy {
                    tenant: domain::Tenant::from(row),
                    copied: copied
                        .into_iter()
                        .filter(|(_, rows)| *rows > 0)
                        .map(|(table, rows)| (table.to_string(), rows))
                        .collect(),
                })
            }
            .scope_boxed()
        })
        .await
    }
}
