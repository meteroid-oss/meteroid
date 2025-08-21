// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ActionAfterTrialEnum"))]
    pub struct ActionAfterTrialEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BankAccountFormat"))]
    pub struct BankAccountFormat;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BillingMetricAggregateEnum"))]
    pub struct BillingMetricAggregateEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BillingPeriodEnum"))]
    pub struct BillingPeriodEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConnectorProviderEnum"))]
    pub struct ConnectorProviderEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConnectorTypeEnum"))]
    pub struct ConnectorTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CreditNoteStatus"))]
    pub struct CreditNoteStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CycleActionEnum"))]
    pub struct CycleActionEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "fang_task_state"))]
    pub struct FangTaskState;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "InvoicePaymentStatus"))]
    pub struct InvoicePaymentStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "InvoiceStatusEnum"))]
    pub struct InvoiceStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "InvoiceType"))]
    pub struct InvoiceType;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "MRRMovementType"))]
    pub struct MrrMovementType;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "OrganizationUserRole"))]
    pub struct OrganizationUserRole;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "PaymentMethodTypeEnum"))]
    pub struct PaymentMethodTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "PaymentStatusEnum"))]
    pub struct PaymentStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "PaymentTypeEnum"))]
    pub struct PaymentTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "PlanStatusEnum"))]
    pub struct PlanStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "PlanTypeEnum"))]
    pub struct PlanTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ScheduledEventStatus"))]
    pub struct ScheduledEventStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ScheduledEventTypeEnum"))]
    pub struct ScheduledEventTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "SubscriptionActivationConditionEnum"))]
    pub struct SubscriptionActivationConditionEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "SubscriptionEventType"))]
    pub struct SubscriptionEventType;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "SubscriptionFeeBillingPeriod"))]
    pub struct SubscriptionFeeBillingPeriod;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "SubscriptionStatusEnum"))]
    pub struct SubscriptionStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "TaxResolverEnum"))]
    pub struct TaxResolverEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "TenantEnvironmentEnum"))]
    pub struct TenantEnvironmentEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "UnitConversionRoundingEnum"))]
    pub struct UnitConversionRoundingEnum;
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
    applied_coupon (id) {
        id -> Uuid,
        coupon_id -> Uuid,
        customer_id -> Uuid,
        subscription_id -> Uuid,
        is_active -> Bool,
        applied_amount -> Nullable<Numeric>,
        applied_count -> Nullable<Int4>,
        last_applied_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BankAccountFormat;

    bank_account (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        currency -> Text,
        country -> Text,
        bank_name -> Text,
        format -> BankAccountFormat,
        account_numbers -> Text,
        created_by -> Uuid,
        created_at -> Timestamp,
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
        product_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ConnectorTypeEnum;
    use super::sql_types::ConnectorProviderEnum;

    connector (id) {
        id -> Uuid,
        created_at -> Timestamp,
        tenant_id -> Uuid,
        alias -> Text,
        connector_type -> ConnectorTypeEnum,
        provider -> ConnectorProviderEnum,
        data -> Nullable<Jsonb>,
        sensitive -> Nullable<Text>,
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
        recurring_value -> Nullable<Int4>,
        reusable -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        redemption_count -> Int4,
        last_redemption_at -> Nullable<Timestamp>,
        disabled -> Bool,
        archived_at -> Nullable<Timestamp>,
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
    custom_tax (id) {
        id -> Uuid,
        invoicing_entity_id -> Uuid,
        name -> Text,
        tax_code -> Text,
        rules -> Jsonb,
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
        alias -> Nullable<Text>,
        billing_email -> Nullable<Text>,
        phone -> Nullable<Text>,
        balance_value_cents -> Int8,
        currency -> Text,
        billing_address -> Nullable<Jsonb>,
        shipping_address -> Nullable<Jsonb>,
        invoicing_entity_id -> Uuid,
        archived_by -> Nullable<Uuid>,
        bank_account_id -> Nullable<Uuid>,
        current_payment_method_id -> Nullable<Uuid>,
        card_provider_id -> Nullable<Uuid>,
        direct_debit_provider_id -> Nullable<Uuid>,
        vat_number -> Nullable<Text>,
        invoicing_emails -> Array<Nullable<Text>>,
        conn_meta -> Nullable<Jsonb>,
        is_tax_exempt -> Bool,
        custom_tax_rate -> Nullable<Numeric>,
        vat_number_format_valid -> Bool,
    }
}

diesel::table! {
    customer_balance_pending_tx (id) {
        id -> Uuid,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        amount_cents -> Int8,
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
        amount_cents -> Int8,
        balance_cents_after -> Int8,
        note -> Nullable<Text>,
        invoice_id -> Nullable<Uuid>,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        created_by -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::PaymentMethodTypeEnum;

    customer_connection (id) {
        id -> Uuid,
        customer_id -> Uuid,
        connector_id -> Uuid,
        supported_payment_types -> Nullable<Array<Nullable<PaymentMethodTypeEnum>>>,
        external_customer_id -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::PaymentMethodTypeEnum;

    customer_payment_method (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        connection_id -> Uuid,
        external_payment_method_id -> Text,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        archived_at -> Nullable<Timestamp>,
        payment_method_type -> PaymentMethodTypeEnum,
        account_number_hint -> Nullable<Text>,
        card_brand -> Nullable<Text>,
        card_last4 -> Nullable<Text>,
        card_exp_month -> Nullable<Int4>,
        card_exp_year -> Nullable<Int4>,
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
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::InvoiceStatusEnum;
    use super::sql_types::InvoiceType;
    use super::sql_types::InvoicePaymentStatus;

    invoice (id) {
        id -> Uuid,
        status -> InvoiceStatusEnum,
        created_at -> Timestamptz,
        updated_at -> Nullable<Timestamptz>,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        subscription_id -> Nullable<Uuid>,
        currency -> Text,
        line_items -> Jsonb,
        data_updated_at -> Nullable<Timestamp>,
        invoice_date -> Date,
        total -> Int8,
        plan_version_id -> Nullable<Uuid>,
        invoice_type -> InvoiceType,
        finalized_at -> Nullable<Timestamp>,
        net_terms -> Int4,
        memo -> Nullable<Text>,
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
        xml_document_id -> Nullable<Uuid>,
        pdf_document_id -> Nullable<Uuid>,
        conn_meta -> Nullable<Jsonb>,
        auto_advance -> Bool,
        issued_at -> Nullable<Timestamptz>,
        payment_status -> InvoicePaymentStatus,
        paid_at -> Nullable<Timestamptz>,
        discount -> Int8,
        purchase_order -> Nullable<Text>,
        tax_breakdown -> Jsonb,
        coupons -> Jsonb,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::TaxResolverEnum;

    invoicing_entity (id) {
        id -> Uuid,
        is_default -> Bool,
        legal_name -> Text,
        invoice_number_pattern -> Text,
        next_invoice_number -> Int8,
        next_credit_note_number -> Int8,
        grace_period_hours -> Int4,
        net_terms -> Int4,
        invoice_footer_info -> Nullable<Text>,
        invoice_footer_legal -> Nullable<Text>,
        logo_attachment_id -> Nullable<Uuid>,
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
        card_provider_id -> Nullable<Uuid>,
        bank_account_id -> Nullable<Uuid>,
        direct_debit_provider_id -> Nullable<Uuid>,
        tax_resolver -> TaxResolverEnum,
    }
}

diesel::table! {
    oauth_verifier (id) {
        id -> Uuid,
        csrf_token -> Text,
        pkce_verifier -> Text,
        created_at -> Timestamp,
        data -> Nullable<Jsonb>,
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
    outbox_event (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        aggregate_id -> Text,
        aggregate_type -> Text,
        event_type -> Text,
        payload -> Nullable<Jsonb>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::PaymentStatusEnum;
    use super::sql_types::PaymentTypeEnum;

    payment_transaction (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        invoice_id -> Uuid,
        provider_transaction_id -> Nullable<Text>,
        processed_at -> Nullable<Timestamp>,
        refunded_at -> Nullable<Timestamp>,
        amount -> Int8,
        currency -> Text,
        payment_method_id -> Nullable<Uuid>,
        status -> PaymentStatusEnum,
        payment_type -> PaymentTypeEnum,
        error_type -> Nullable<Text>,
        receipt_pdf_id -> Nullable<Uuid>,
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
        plan_type -> PlanTypeEnum,
        status -> PlanStatusEnum,
        active_version_id -> Nullable<Uuid>,
        draft_version_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ActionAfterTrialEnum;

    plan_version (id) {
        id -> Uuid,
        is_draft_version -> Bool,
        plan_id -> Uuid,
        version -> Int4,
        trial_duration_days -> Nullable<Int4>,
        downgrade_plan_id -> Nullable<Uuid>,
        tenant_id -> Uuid,
        period_start_day -> Nullable<Int2>,
        net_terms -> Int4,
        currency -> Text,
        billing_cycles -> Nullable<Int4>,
        created_at -> Timestamp,
        created_by -> Uuid,
        trialing_plan_id -> Nullable<Uuid>,
        action_after_trial -> Nullable<ActionAfterTrialEnum>,
        trial_is_free -> Bool,
    }
}

diesel::table! {
    price_component (id) {
        id -> Uuid,
        name -> Text,
        fee -> Jsonb,
        plan_version_id -> Uuid,
        product_id -> Nullable<Uuid>,
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
    product_accounting (product_id, invoicing_entity_id) {
        product_id -> Uuid,
        invoicing_entity_id -> Uuid,
        custom_tax_id -> Nullable<Uuid>,
        product_code -> Nullable<Text>,
        ledger_account_code -> Nullable<Text>,
    }
}

diesel::table! {
    product_family (id) {
        id -> Uuid,
        name -> Text,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        tenant_id -> Uuid,
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
    use diesel::sql_types::*;
    use super::sql_types::ScheduledEventTypeEnum;
    use super::sql_types::ScheduledEventStatus;

    scheduled_event (id) {
        id -> Uuid,
        subscription_id -> Uuid,
        tenant_id -> Uuid,
        event_type -> ScheduledEventTypeEnum,
        scheduled_time -> Timestamp,
        priority -> Int4,
        event_data -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        status -> ScheduledEventStatus,
        retries -> Int4,
        last_retry_at -> Nullable<Timestamp>,
        error -> Nullable<Text>,
        processed_at -> Nullable<Timestamp>,
        source -> Text,
    }
}

diesel::table! {
    slot_transaction (id) {
        id -> Uuid,
        subscription_id -> Uuid,
        delta -> Int4,
        prev_active_slots -> Int4,
        effective_at -> Timestamp,
        transaction_at -> Timestamp,
        #[max_length = 255]
        unit -> Varchar,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BillingPeriodEnum;
    use super::sql_types::PaymentMethodTypeEnum;
    use super::sql_types::SubscriptionActivationConditionEnum;
    use super::sql_types::SubscriptionStatusEnum;
    use super::sql_types::CycleActionEnum;

    subscription (id) {
        id -> Uuid,
        customer_id -> Uuid,
        billing_day_anchor -> Int2,
        tenant_id -> Uuid,
        start_date -> Date,
        plan_version_id -> Uuid,
        created_at -> Timestamp,
        created_by -> Uuid,
        net_terms -> Int4,
        invoice_memo -> Nullable<Text>,
        invoice_threshold -> Nullable<Numeric>,
        activated_at -> Nullable<Timestamp>,
        #[max_length = 3]
        currency -> Varchar,
        mrr_cents -> Int8,
        period -> BillingPeriodEnum,
        card_connection_id -> Nullable<Uuid>,
        direct_debit_connection_id -> Nullable<Uuid>,
        bank_account_id -> Nullable<Uuid>,
        pending_checkout -> Bool,
        payment_method_type -> Nullable<PaymentMethodTypeEnum>,
        payment_method -> Nullable<Uuid>,
        end_date -> Nullable<Date>,
        trial_duration -> Nullable<Int4>,
        activation_condition -> SubscriptionActivationConditionEnum,
        billing_start_date -> Nullable<Date>,
        conn_meta -> Nullable<Jsonb>,
        cycle_index -> Nullable<Int4>,
        status -> SubscriptionStatusEnum,
        current_period_start -> Date,
        current_period_end -> Nullable<Date>,
        next_cycle_action -> Nullable<CycleActionEnum>,
        last_error -> Nullable<Text>,
        error_count -> Int4,
        next_retry -> Nullable<Timestamp>,
        auto_advance_invoices -> Bool,
        charge_automatically -> Bool,
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
        product_id -> Nullable<Uuid>,
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
        reporting_currency -> Text,
        environment -> TenantEnvironmentEnum,
        available_currencies -> Array<Nullable<Text>>,
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

diesel::joinable!(add_on -> tenant (tenant_id));
diesel::joinable!(api_token -> tenant (tenant_id));
diesel::joinable!(applied_coupon -> coupon (coupon_id));
diesel::joinable!(applied_coupon -> customer (customer_id));
diesel::joinable!(applied_coupon -> subscription (subscription_id));
diesel::joinable!(bank_account -> tenant (tenant_id));
diesel::joinable!(bi_delta_mrr_daily -> historical_rates_from_usd (historical_rate_id));
diesel::joinable!(bi_mrr_movement_log -> credit_note (credit_note_id));
diesel::joinable!(bi_mrr_movement_log -> invoice (invoice_id));
diesel::joinable!(bi_mrr_movement_log -> plan_version (plan_version_id));
diesel::joinable!(bi_mrr_movement_log -> tenant (tenant_id));
diesel::joinable!(bi_revenue_daily -> historical_rates_from_usd (historical_rate_id));
diesel::joinable!(billable_metric -> product (product_id));
diesel::joinable!(billable_metric -> product_family (product_family_id));
diesel::joinable!(billable_metric -> tenant (tenant_id));
diesel::joinable!(coupon -> tenant (tenant_id));
diesel::joinable!(credit_note -> customer (customer_id));
diesel::joinable!(credit_note -> invoice (invoice_id));
diesel::joinable!(credit_note -> plan_version (plan_version_id));
diesel::joinable!(credit_note -> tenant (tenant_id));
diesel::joinable!(custom_tax -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(customer -> bank_account (bank_account_id));
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
diesel::joinable!(customer_connection -> connector (connector_id));
diesel::joinable!(customer_connection -> customer (customer_id));
diesel::joinable!(customer_payment_method -> customer_connection (connection_id));
diesel::joinable!(customer_payment_method -> tenant (tenant_id));
diesel::joinable!(invoice -> customer (customer_id));
diesel::joinable!(invoice -> plan_version (plan_version_id));
diesel::joinable!(invoice -> tenant (tenant_id));
diesel::joinable!(invoicing_entity -> bank_account (bank_account_id));
diesel::joinable!(invoicing_entity -> tenant (tenant_id));
diesel::joinable!(organization_member -> organization (organization_id));
diesel::joinable!(organization_member -> user (user_id));
diesel::joinable!(payment_transaction -> customer_payment_method (payment_method_id));
diesel::joinable!(payment_transaction -> invoice (invoice_id));
diesel::joinable!(payment_transaction -> tenant (tenant_id));
diesel::joinable!(plan -> product_family (product_family_id));
diesel::joinable!(plan -> tenant (tenant_id));
diesel::joinable!(price_component -> billable_metric (billable_metric_id));
diesel::joinable!(price_component -> plan_version (plan_version_id));
diesel::joinable!(price_component -> product (product_id));
diesel::joinable!(product -> product_family (product_family_id));
diesel::joinable!(product -> tenant (tenant_id));
diesel::joinable!(product_accounting -> custom_tax (custom_tax_id));
diesel::joinable!(product_accounting -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(product_accounting -> product (product_id));
diesel::joinable!(product_family -> tenant (tenant_id));
diesel::joinable!(schedule -> plan_version (plan_version_id));
diesel::joinable!(scheduled_event -> subscription (subscription_id));
diesel::joinable!(slot_transaction -> subscription (subscription_id));
diesel::joinable!(subscription -> bank_account (bank_account_id));
diesel::joinable!(subscription -> customer (customer_id));
diesel::joinable!(subscription -> customer_payment_method (payment_method));
diesel::joinable!(subscription -> plan_version (plan_version_id));
diesel::joinable!(subscription -> tenant (tenant_id));
diesel::joinable!(subscription_add_on -> add_on (add_on_id));
diesel::joinable!(subscription_add_on -> subscription (subscription_id));
diesel::joinable!(subscription_component -> price_component (price_component_id));
diesel::joinable!(subscription_component -> product (product_id));
diesel::joinable!(subscription_component -> subscription (subscription_id));
diesel::joinable!(subscription_event -> bi_mrr_movement_log (bi_mrr_movement_log_id));
diesel::joinable!(subscription_event -> subscription (subscription_id));
diesel::joinable!(tenant -> organization (organization_id));
diesel::joinable!(webhook_in_event -> connector (provider_config_id));

diesel::allow_tables_to_appear_in_same_query!(
    add_on,
    api_token,
    applied_coupon,
    bank_account,
    bi_customer_ytd_summary,
    bi_delta_mrr_daily,
    bi_mrr_movement_log,
    bi_revenue_daily,
    billable_metric,
    connector,
    coupon,
    credit_note,
    custom_tax,
    customer,
    customer_balance_pending_tx,
    customer_balance_tx,
    customer_connection,
    customer_payment_method,
    fang_tasks,
    fang_tasks_archive,
    historical_rates_from_usd,
    invoice,
    invoicing_entity,
    oauth_verifier,
    organization,
    organization_member,
    outbox_event,
    payment_transaction,
    plan,
    plan_version,
    price_component,
    product,
    product_accounting,
    product_family,
    schedule,
    scheduled_event,
    slot_transaction,
    subscription,
    subscription_add_on,
    subscription_component,
    subscription_event,
    tenant,
    user,
    webhook_in_event,
);
