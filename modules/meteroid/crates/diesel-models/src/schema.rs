// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BillingMetricAggregateEnum"))]
    pub struct BillingMetricAggregateEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BillingPeriodEnum"))]
    pub struct BillingPeriodEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CreditNoteStatus"))]
    pub struct CreditNoteStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "fang_task_state"))]
    pub struct FangTaskState;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "InvoiceExternalStatusEnum"))]
    pub struct InvoiceExternalStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "InvoiceStatusEnum"))]
    pub struct InvoiceStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "InvoiceType"))]
    pub struct InvoiceType;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "InvoicingProviderEnum"))]
    pub struct InvoicingProviderEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "MRRMovementType"))]
    pub struct MrrMovementType;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "OutboxStatus"))]
    pub struct OutboxStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "OrganizationUserRole"))]
    pub struct OrganizationUserRole;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "PlanStatusEnum"))]
    pub struct PlanStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "PlanTypeEnum"))]
    pub struct PlanTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "SubscriptionEventType"))]
    pub struct SubscriptionEventType;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "SubscriptionFeeBillingPeriod"))]
    pub struct SubscriptionFeeBillingPeriod;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "TenantEnvironmentEnum"))]
    pub struct TenantEnvironmentEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "UnitConversionRoundingEnum"))]
    pub struct UnitConversionRoundingEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "WebhookOutEventTypeEnum"))]
    pub struct WebhookOutEventTypeEnum;
}

diesel::table! {
    add_on (id) {
        id -> Uuid,
        name -> Text,
        fee -> Jsonb,
        tenant_id -> Uuid,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    api_token (id) {
        id -> Uuid,
        name -> Text,
        created_at -> Timestamp,
        created_by -> Uuid,
        tenant_id -> Uuid,
        hash -> Text,
        hint -> Text,
    }
}

diesel::table! {
    bi_customer_ytd_summary (tenant_id, customer_id, currency, revenue_year) {
        tenant_id -> Uuid,
        customer_id -> Uuid,
        revenue_year -> Int4,
        currency -> Text,
        total_revenue_cents -> Int8,
    }
}

diesel::table! {
    bi_delta_mrr_daily (tenant_id, plan_version_id, currency, date) {
        tenant_id -> Uuid,
        plan_version_id -> Uuid,
        date -> Date,
        currency -> Text,
        net_mrr_cents -> Int8,
        new_business_cents -> Int8,
        new_business_count -> Int4,
        expansion_cents -> Int8,
        expansion_count -> Int4,
        contraction_cents -> Int8,
        contraction_count -> Int4,
        churn_cents -> Int8,
        churn_count -> Int4,
        reactivation_cents -> Int8,
        reactivation_count -> Int4,
        historical_rate_id -> Uuid,
        net_mrr_cents_usd -> Int8,
        new_business_cents_usd -> Int8,
        expansion_cents_usd -> Int8,
        contraction_cents_usd -> Int8,
        churn_cents_usd -> Int8,
        reactivation_cents_usd -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::MrrMovementType;

    bi_mrr_movement_log (id) {
        id -> Uuid,
        description -> Text,
        movement_type -> MrrMovementType,
        net_mrr_change -> Int8,
        #[max_length = 3]
        currency -> Varchar,
        created_at -> Timestamp,
        applies_to -> Date,
        invoice_id -> Uuid,
        credit_note_id -> Nullable<Uuid>,
        plan_version_id -> Uuid,
        tenant_id -> Uuid,
    }
}

diesel::table! {
    bi_revenue_daily (id) {
        tenant_id -> Uuid,
        plan_version_id -> Nullable<Uuid>,
        currency -> Text,
        revenue_date -> Date,
        net_revenue_cents -> Int8,
        historical_rate_id -> Uuid,
        net_revenue_cents_usd -> Int8,
        id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BillingMetricAggregateEnum;
    use super::sql_types::UnitConversionRoundingEnum;

    billable_metric (id) {
        id -> Uuid,
        name -> Text,
        description -> Nullable<Text>,
        code -> Text,
        aggregation_type -> BillingMetricAggregateEnum,
        aggregation_key -> Nullable<Text>,
        unit_conversion_factor -> Nullable<Int4>,
        unit_conversion_rounding -> Nullable<UnitConversionRoundingEnum>,
        segmentation_matrix -> Nullable<Jsonb>,
        usage_group_key -> Nullable<Text>,
        created_at -> Timestamp,
        created_by -> Uuid,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        tenant_id -> Uuid,
        product_family_id -> Uuid,
    }
}

diesel::table! {

    use diesel::sql_types::*;
    use super::sql_types::OutboxStatus;

    outbox (id) {
        id -> Uuid,
        event_type -> Text,
        resource_id -> Uuid,
        status -> OutboxStatus,
        payload -> Nullable<Jsonb>,
        created_at -> Timestamp,
        processing_started_at -> Nullable<Timestamp>,
        processing_completed_at -> Nullable<Timestamp>,
        processing_attempts -> Int4,
        error -> Nullable<Text>,
    }
}

diesel::table! {
    coupon (id) {
        id -> Uuid,
        code -> Text,
        description -> Text,
        tenant_id -> Uuid,
        discount -> Jsonb,
        expires_at -> Nullable<Timestamp>,
        redemption_limit -> Nullable<Int4>,
        recurring_value -> Int4,
        reusable -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CreditNoteStatus;

    credit_note (id) {
        id -> Uuid,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        refunded_amount_cents -> Nullable<Int8>,
        credited_amount_cents -> Nullable<Int8>,
        currency -> Text,
        finalized_at -> Timestamp,
        plan_version_id -> Nullable<Uuid>,
        invoice_id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        status -> CreditNoteStatus,
    }
}

diesel::table! {
    customer (id) {
        id -> Uuid,
        name -> Text,
        created_at -> Timestamp,
        created_by -> Uuid,
        updated_at -> Nullable<Timestamp>,
        updated_by -> Nullable<Uuid>,
        archived_at -> Nullable<Timestamp>,
        tenant_id -> Uuid,
        billing_config -> Jsonb,
        alias -> Nullable<Text>,
        email -> Nullable<Text>,
        invoicing_email -> Nullable<Text>,
        phone -> Nullable<Text>,
        balance_value_cents -> Int4,
        currency -> Text,
        billing_address -> Nullable<Jsonb>,
        shipping_address -> Nullable<Jsonb>,
        invoicing_entity_id -> Uuid,
    }
}

diesel::table! {
    customer_balance_pending_tx (id) {
        id -> Uuid,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        amount_cents -> Int4,
        note -> Nullable<Text>,
        invoice_id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        tx_id -> Nullable<Uuid>,
        created_by -> Uuid,
    }
}

diesel::table! {
    customer_balance_tx (id) {
        id -> Uuid,
        created_at -> Timestamp,
        amount_cents -> Int4,
        balance_cents_after -> Int4,
        note -> Nullable<Text>,
        invoice_id -> Nullable<Uuid>,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        created_by -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::FangTaskState;

    fang_tasks (id) {
        id -> Uuid,
        metadata -> Jsonb,
        error_message -> Nullable<Text>,
        state -> FangTaskState,
        task_type -> Varchar,
        #[max_length = 64]
        uniq_hash -> Nullable<Bpchar>,
        retries -> Int4,
        scheduled_at -> Timestamptz,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::FangTaskState;

    fang_tasks_archive (id) {
        id -> Uuid,
        metadata -> Jsonb,
        error_message -> Nullable<Text>,
        state -> FangTaskState,
        task_type -> Varchar,
        #[max_length = 64]
        uniq_hash -> Nullable<Bpchar>,
        retries -> Int4,
        scheduled_at -> Timestamptz,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        archived_at -> Timestamptz,
    }
}

diesel::table! {
    historical_rates_from_usd (id) {
        id -> Uuid,
        date -> Date,
        rates -> Jsonb,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::InvoiceStatusEnum;
    use super::sql_types::InvoiceExternalStatusEnum;
    use super::sql_types::InvoicingProviderEnum;
    use super::sql_types::InvoiceType;

    invoice (id) {
        id -> Uuid,
        status -> InvoiceStatusEnum,
        external_status -> Nullable<InvoiceExternalStatusEnum>,
        created_at -> Timestamptz,
        updated_at -> Nullable<Timestamptz>,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        subscription_id -> Nullable<Uuid>,
        currency -> Text,
        external_invoice_id -> Nullable<Text>,
        invoicing_provider -> InvoicingProviderEnum,
        line_items -> Jsonb,
        issued -> Bool,
        issue_attempts -> Int4,
        last_issue_attempt_at -> Nullable<Timestamptz>,
        last_issue_error -> Nullable<Text>,
        data_updated_at -> Nullable<Timestamp>,
        invoice_date -> Date,
        total -> Int8,
        plan_version_id -> Nullable<Uuid>,
        invoice_type -> InvoiceType,
        finalized_at -> Nullable<Timestamp>,
        net_terms -> Int4,
        memo -> Nullable<Text>,
        tax_rate -> Int4,
        local_id -> Text,
        reference -> Nullable<Text>,
        invoice_number -> Text,
        tax_amount -> Int8,
        subtotal_recurring -> Int8,
        plan_name -> Nullable<Text>,
        due_at -> Nullable<Timestamp>,
        customer_details -> Jsonb,
        amount_due -> Int8,
        subtotal -> Int8,
        applied_credits -> Int8,
        seller_details -> Jsonb,
        pdf_document_id -> Nullable<Text>,
        xml_document_id -> Nullable<Text>,
    }
}

diesel::table! {
    invoicing_entity (id) {
        id -> Uuid,
        local_id -> Text,
        is_default -> Bool,
        legal_name -> Text,
        invoice_number_pattern -> Text,
        next_invoice_number -> Int8,
        next_credit_note_number -> Int8,
        grace_period_hours -> Int4,
        net_terms -> Int4,
        invoice_footer_info -> Nullable<Text>,
        invoice_footer_legal -> Nullable<Text>,
        logo_attachment_id -> Nullable<Text>,
        brand_color -> Nullable<Text>,
        address_line1 -> Nullable<Text>,
        address_line2 -> Nullable<Text>,
        #[max_length = 50]
        zip_code -> Nullable<Varchar>,
        state -> Nullable<Text>,
        city -> Nullable<Text>,
        vat_number -> Nullable<Text>,
        country -> Text,
        #[max_length = 50]
        accounting_currency -> Varchar,
        tenant_id -> Uuid,
    }
}

diesel::table! {
    organization (id) {
        id -> Uuid,
        trade_name -> Text,
        slug -> Text,
        created_at -> Timestamp,
        archived_at -> Nullable<Timestamp>,
        invite_link_hash -> Nullable<Text>,
        default_country -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::OrganizationUserRole;

    organization_member (user_id, organization_id) {
        user_id -> Uuid,
        organization_id -> Uuid,
        role -> OrganizationUserRole,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::PlanTypeEnum;
    use super::sql_types::PlanStatusEnum;

    plan (id) {
        id -> Uuid,
        name -> Text,
        description -> Nullable<Text>,
        created_at -> Timestamp,
        created_by -> Uuid,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        tenant_id -> Uuid,
        product_family_id -> Uuid,
        external_id -> Text,
        plan_type -> PlanTypeEnum,
        status -> PlanStatusEnum,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BillingPeriodEnum;

    plan_version (id) {
        id -> Uuid,
        is_draft_version -> Bool,
        plan_id -> Uuid,
        version -> Int4,
        trial_duration_days -> Nullable<Int4>,
        trial_fallback_plan_id -> Nullable<Uuid>,
        tenant_id -> Uuid,
        period_start_day -> Nullable<Int2>,
        net_terms -> Int4,
        currency -> Text,
        billing_cycles -> Nullable<Int4>,
        created_at -> Timestamp,
        created_by -> Uuid,
        billing_periods -> Array<Nullable<BillingPeriodEnum>>,
    }
}

diesel::table! {
    price_component (id) {
        id -> Uuid,
        name -> Text,
        fee -> Jsonb,
        plan_version_id -> Uuid,
        product_item_id -> Nullable<Uuid>,
        billable_metric_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    product (id) {
        id -> Uuid,
        name -> Text,
        description -> Nullable<Text>,
        created_at -> Timestamp,
        created_by -> Uuid,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        tenant_id -> Uuid,
        product_family_id -> Uuid,
    }
}

diesel::table! {
    product_family (id) {
        id -> Uuid,
        name -> Text,
        external_id -> Text,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        tenant_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::InvoicingProviderEnum;

    provider_config (id) {
        id -> Uuid,
        created_at -> Timestamp,
        tenant_id -> Uuid,
        invoicing_provider -> InvoicingProviderEnum,
        enabled -> Bool,
        webhook_security -> Jsonb,
        api_security -> Jsonb,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BillingPeriodEnum;

    schedule (id) {
        id -> Uuid,
        billing_period -> BillingPeriodEnum,
        plan_version_id -> Uuid,
        ramps -> Jsonb,
    }
}

diesel::table! {
    slot_transaction (id) {
        id -> Uuid,
        price_component_id -> Uuid,
        subscription_id -> Uuid,
        delta -> Int4,
        prev_active_slots -> Int4,
        effective_at -> Timestamp,
        transaction_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BillingPeriodEnum;

    subscription (id) {
        id -> Uuid,
        customer_id -> Uuid,
        billing_day -> Int2,
        tenant_id -> Uuid,
        trial_start_date -> Nullable<Date>,
        billing_start_date -> Date,
        billing_end_date -> Nullable<Date>,
        plan_version_id -> Uuid,
        created_at -> Timestamp,
        created_by -> Uuid,
        net_terms -> Int4,
        invoice_memo -> Nullable<Text>,
        invoice_threshold -> Nullable<Numeric>,
        activated_at -> Nullable<Timestamp>,
        canceled_at -> Nullable<Timestamp>,
        cancellation_reason -> Nullable<Text>,
        #[max_length = 3]
        currency -> Varchar,
        mrr_cents -> Int8,
        period -> BillingPeriodEnum,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SubscriptionFeeBillingPeriod;

    subscription_add_on (id) {
        id -> Uuid,
        name -> Text,
        subscription_id -> Uuid,
        add_on_id -> Uuid,
        period -> SubscriptionFeeBillingPeriod,
        fee -> Jsonb,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SubscriptionFeeBillingPeriod;

    subscription_component (id) {
        id -> Uuid,
        name -> Text,
        subscription_id -> Uuid,
        price_component_id -> Nullable<Uuid>,
        product_item_id -> Nullable<Uuid>,
        period -> SubscriptionFeeBillingPeriod,
        fee -> Jsonb,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SubscriptionEventType;

    subscription_event (id) {
        id -> Uuid,
        mrr_delta -> Nullable<Int8>,
        event_type -> SubscriptionEventType,
        created_at -> Timestamp,
        applies_to -> Date,
        subscription_id -> Uuid,
        bi_mrr_movement_log_id -> Nullable<Uuid>,
        details -> Nullable<Jsonb>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::TenantEnvironmentEnum;

    tenant (id) {
        id -> Uuid,
        name -> Text,
        slug -> Text,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        organization_id -> Uuid,
        currency -> Text,
        environment -> TenantEnvironmentEnum,
    }
}

diesel::table! {
    user (id) {
        id -> Uuid,
        email -> Text,
        created_at -> Timestamp,
        archived_at -> Nullable<Timestamp>,
        password_hash -> Nullable<Text>,
        onboarded -> Bool,
        first_name -> Nullable<Text>,
        last_name -> Nullable<Text>,
        department -> Nullable<Text>,
    }
}

diesel::table! {
    webhook_in_event (id) {
        id -> Uuid,
        received_at -> Timestamptz,
        action -> Nullable<Text>,
        key -> Text,
        processed -> Bool,
        attempts -> Int4,
        error -> Nullable<Text>,
        provider_config_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::WebhookOutEventTypeEnum;

    webhook_out_endpoint (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        url -> Text,
        description -> Nullable<Text>,
        secret -> Text,
        created_at -> Timestamp,
        events_to_listen -> Array<Nullable<WebhookOutEventTypeEnum>>,
        enabled -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::WebhookOutEventTypeEnum;

    webhook_out_event (id) {
        id -> Uuid,
        endpoint_id -> Uuid,
        created_at -> Timestamp,
        event_type -> WebhookOutEventTypeEnum,
        request_body -> Text,
        response_body -> Nullable<Text>,
        http_status_code -> Nullable<Int2>,
        error_message -> Nullable<Text>,
    }
}

diesel::joinable!(add_on -> tenant (tenant_id));
diesel::joinable!(api_token -> tenant (tenant_id));
diesel::joinable!(bi_delta_mrr_daily -> historical_rates_from_usd (historical_rate_id));
diesel::joinable!(bi_mrr_movement_log -> credit_note (credit_note_id));
diesel::joinable!(bi_mrr_movement_log -> invoice (invoice_id));
diesel::joinable!(bi_mrr_movement_log -> plan_version (plan_version_id));
diesel::joinable!(bi_mrr_movement_log -> tenant (tenant_id));
diesel::joinable!(bi_revenue_daily -> historical_rates_from_usd (historical_rate_id));
diesel::joinable!(billable_metric -> product_family (product_family_id));
diesel::joinable!(billable_metric -> tenant (tenant_id));
diesel::joinable!(coupon -> tenant (tenant_id));
diesel::joinable!(credit_note -> customer (customer_id));
diesel::joinable!(credit_note -> invoice (invoice_id));
diesel::joinable!(credit_note -> plan_version (plan_version_id));
diesel::joinable!(credit_note -> tenant (tenant_id));
diesel::joinable!(customer -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(customer -> tenant (tenant_id));
diesel::joinable!(customer_balance_pending_tx -> customer (customer_id));
diesel::joinable!(customer_balance_pending_tx -> customer_balance_tx (tx_id));
diesel::joinable!(customer_balance_pending_tx -> invoice (invoice_id));
diesel::joinable!(customer_balance_pending_tx -> tenant (tenant_id));
diesel::joinable!(customer_balance_pending_tx -> user (created_by));
diesel::joinable!(customer_balance_tx -> customer (customer_id));
diesel::joinable!(customer_balance_tx -> invoice (invoice_id));
diesel::joinable!(customer_balance_tx -> tenant (tenant_id));
diesel::joinable!(customer_balance_tx -> user (created_by));
diesel::joinable!(invoice -> customer (customer_id));
diesel::joinable!(invoice -> plan_version (plan_version_id));
diesel::joinable!(invoice -> tenant (tenant_id));
diesel::joinable!(invoicing_entity -> tenant (tenant_id));
diesel::joinable!(organization_member -> organization (organization_id));
diesel::joinable!(organization_member -> user (user_id));
diesel::joinable!(plan -> product_family (product_family_id));
diesel::joinable!(plan -> tenant (tenant_id));
diesel::joinable!(plan_version -> plan (plan_id));
diesel::joinable!(price_component -> billable_metric (billable_metric_id));
diesel::joinable!(price_component -> plan_version (plan_version_id));
diesel::joinable!(price_component -> product (product_item_id));
diesel::joinable!(product -> product_family (product_family_id));
diesel::joinable!(product -> tenant (tenant_id));
diesel::joinable!(product_family -> tenant (tenant_id));
diesel::joinable!(schedule -> plan_version (plan_version_id));
diesel::joinable!(slot_transaction -> price_component (price_component_id));
diesel::joinable!(slot_transaction -> subscription (subscription_id));
diesel::joinable!(subscription -> customer (customer_id));
diesel::joinable!(subscription -> plan_version (plan_version_id));
diesel::joinable!(subscription -> tenant (tenant_id));
diesel::joinable!(subscription_add_on -> add_on (add_on_id));
diesel::joinable!(subscription_add_on -> subscription (subscription_id));
diesel::joinable!(subscription_component -> price_component (price_component_id));
diesel::joinable!(subscription_component -> product (product_item_id));
diesel::joinable!(subscription_component -> subscription (subscription_id));
diesel::joinable!(subscription_event -> bi_mrr_movement_log (bi_mrr_movement_log_id));
diesel::joinable!(subscription_event -> subscription (subscription_id));
diesel::joinable!(tenant -> organization (organization_id));
diesel::joinable!(webhook_in_event -> provider_config (provider_config_id));
diesel::joinable!(webhook_out_endpoint -> tenant (tenant_id));
diesel::joinable!(webhook_out_event -> webhook_out_endpoint (endpoint_id));

diesel::allow_tables_to_appear_in_same_query!(
    add_on,
    api_token,
    bi_customer_ytd_summary,
    bi_delta_mrr_daily,
    bi_mrr_movement_log,
    bi_revenue_daily,
    billable_metric,
    coupon,
    credit_note,
    customer,
    customer_balance_pending_tx,
    customer_balance_tx,
    fang_tasks,
    fang_tasks_archive,
    historical_rates_from_usd,
    invoice,
    invoicing_entity,
    outbox,
    organization,
    organization_member,
    plan,
    plan_version,
    price_component,
    product,
    product_family,
    provider_config,
    schedule,
    slot_transaction,
    subscription,
    subscription_add_on,
    subscription_component,
    subscription_event,
    tenant,
    user,
    webhook_in_event,
    webhook_out_endpoint,
    webhook_out_event,
);
