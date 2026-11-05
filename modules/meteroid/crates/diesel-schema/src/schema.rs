// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ActorTypeEnum"))]
    pub struct ActorTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "AdjustmentKindEnum"))]
    pub struct AdjustmentKindEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "AlertConditionKindEnum"))]
    pub struct AlertConditionKindEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "AlertEventStatusEnum"))]
    pub struct AlertEventStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "AlertSeverityEnum"))]
    pub struct AlertSeverityEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BankAccountFormat"))]
    pub struct BankAccountFormat;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BatchJobChunkStatusEnum"))]
    pub struct BatchJobChunkStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BatchJobStatusEnum"))]
    pub struct BatchJobStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BatchJobTypeEnum"))]
    pub struct BatchJobTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BillingMetricAggregateEnum"))]
    pub struct BillingMetricAggregateEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BillingPeriodEnum"))]
    pub struct BillingPeriodEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BudgetAuthorEnum"))]
    pub struct BudgetAuthorEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "BudgetBehaviorEnum"))]
    pub struct BudgetBehaviorEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ChannelTypeEnum"))]
    pub struct ChannelTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CheckoutSessionStatusEnum"))]
    pub struct CheckoutSessionStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CheckoutTypeEnum"))]
    pub struct CheckoutTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConfigValueTypeEnum"))]
    pub struct ConfigValueTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConnectConnectionStatusEnum"))]
    pub struct ConnectConnectionStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConnectConnectionTypeEnum"))]
    pub struct ConnectConnectionTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConnectOnboardingModeEnum"))]
    pub struct ConnectOnboardingModeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConnectorProviderEnum"))]
    pub struct ConnectorProviderEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConnectorStatusEnum"))]
    pub struct ConnectorStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConnectorTypeEnum"))]
    pub struct ConnectorTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ConnectorWebhookRouteEnum"))]
    pub struct ConnectorWebhookRouteEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CreditExhaustedRuleEnum"))]
    pub struct CreditExhaustedRuleEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CreditInputChangeKindEnum"))]
    pub struct CreditInputChangeKindEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CreditNoteStatus"))]
    pub struct CreditNoteStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CreditTypeEnum"))]
    pub struct CreditTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CustomPropertyEntityTypeEnum"))]
    pub struct CustomPropertyEntityTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CustomPropertyTypeEnum"))]
    pub struct CustomPropertyTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CustomerMilestoneEnum"))]
    pub struct CustomerMilestoneEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CustomerTaxStatusEnum"))]
    pub struct CustomerTaxStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CustomerTypeEnum"))]
    pub struct CustomerTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CustomerVatValidationStatusEnum"))]
    pub struct CustomerVatValidationStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "CycleActionEnum"))]
    pub struct CycleActionEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "DeadLetterStatusEnum"))]
    pub struct DeadLetterStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "DeclineKindEnum"))]
    pub struct DeclineKindEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "EInvoicingFormatEnum"))]
    pub struct EInvoicingFormatEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "EInvoicingStatusEnum"))]
    pub struct EInvoicingStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "EmailDeliveryModeEnum"))]
    pub struct EmailDeliveryModeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "EmailKindAudienceModeEnum"))]
    pub struct EmailKindAudienceModeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "EmailSendClaimStateEnum"))]
    pub struct EmailSendClaimStateEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "EntitlementEntityTypeEnum"))]
    pub struct EntitlementEntityTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "EntitlementModeEnum"))]
    pub struct EntitlementModeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "EntityListTypeEnum"))]
    pub struct EntityListTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "FeatureStatusEnum"))]
    pub struct FeatureStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "FeatureTypeEnum"))]
    pub struct FeatureTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "FeeTypeEnum"))]
    pub struct FeeTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "InvoiceAnnexKindEnum"))]
    pub struct InvoiceAnnexKindEnum;

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
    #[diesel(postgres_type(name = "MetricTargetChangeKindEnum"))]
    pub struct MetricTargetChangeKindEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "MetricTargetPeriodGrainEnum"))]
    pub struct MetricTargetPeriodGrainEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "MilestoneEnum"))]
    pub struct MilestoneEnum;

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
    #[diesel(postgres_type(name = "PaymentReminderStepActionEnum"))]
    pub struct PaymentReminderStepActionEnum;

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
    #[diesel(postgres_type(name = "QuoteStatusEnum"))]
    pub struct QuoteStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "RefundModeEnum"))]
    pub struct RefundModeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ReminderEnrolledViaEnum"))]
    pub struct ReminderEnrolledViaEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ReminderNotEnrolledReasonEnum"))]
    pub struct ReminderNotEnrolledReasonEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ReminderStateEnum"))]
    pub struct ReminderStateEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ReminderStopReasonEnum"))]
    pub struct ReminderStopReasonEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ReversalKindEnum"))]
    pub struct ReversalKindEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "SandboxPoolStatusEnum"))]
    pub struct SandboxPoolStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "SandboxSeedStatusEnum"))]
    pub struct SandboxSeedStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ScheduledEventStatus"))]
    pub struct ScheduledEventStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "ScheduledEventTypeEnum"))]
    pub struct ScheduledEventTypeEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "slot_transaction_status"))]
    pub struct SlotTransactionStatus;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "StoredObjectKindEnum"))]
    pub struct StoredObjectKindEnum;

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
    #[diesel(postgres_type(name = "TaxIdCollectionEnum"))]
    pub struct TaxIdCollectionEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "TaxResolverEnum"))]
    pub struct TaxResolverEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "TenantEnvironmentEnum"))]
    pub struct TenantEnvironmentEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "TenantStatusEnum"))]
    pub struct TenantStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "UnitConversionRoundingEnum"))]
    pub struct UnitConversionRoundingEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "UsageAtZeroEnum"))]
    pub struct UsageAtZeroEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "WebhookDeliveryStatusEnum"))]
    pub struct WebhookDeliveryStatusEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "WebhookEndpointDisabledReasonEnum"))]
    pub struct WebhookEndpointDisabledReasonEnum;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "xid8", schema = "pg_catalog"))]
    pub struct Xid8;
}

diesel::table! {
    add_on (id) {
        id -> Uuid,
        name -> Text,
        tenant_id -> Uuid,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        product_id -> Uuid,
        price_id -> Uuid,
        description -> Nullable<Text>,
        self_serviceable -> Bool,
        max_instances_per_subscription -> Nullable<Int4>,
        archived_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::AlertConditionKindEnum;
    use super::sql_types::AlertSeverityEnum;

    alert_definition (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        name -> Text,
        metric_key -> Text,
        condition_kind -> AlertConditionKindEnum,
        condition -> Jsonb,
        grain -> Text,
        filters -> Nullable<Jsonb>,
        cohort -> Nullable<Jsonb>,
        channels -> Jsonb,
        severity -> AlertSeverityEnum,
        cooldown_secs -> Int4,
        enabled -> Bool,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        kind -> Text,
        last_evaluated_at -> Nullable<Timestamp>,
        last_active -> Nullable<Bool>,
        last_value -> Nullable<Float8>,
        last_impact_cents -> Nullable<Int8>,
        last_preview -> Nullable<Jsonb>,
        builtin_code -> Nullable<Text>,
        settings -> Nullable<Jsonb>,
        builtin_cursor -> Nullable<Uuid>,
        category -> Nullable<Text>,
        owner_user_id -> Nullable<Uuid>,
        muted_until -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::AlertSeverityEnum;
    use super::sql_types::AlertEventStatusEnum;

    alert_event (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        alert_definition_id -> Uuid,
        fired_at -> Timestamp,
        window_date -> Date,
        metric_value -> Float8,
        baseline_value -> Nullable<Float8>,
        score -> Nullable<Float8>,
        severity -> AlertSeverityEnum,
        status -> AlertEventStatusEnum,
        payload -> Jsonb,
        delivered_at -> Nullable<Timestamp>,
        acknowledged_at -> Nullable<Timestamp>,
        customer_id -> Nullable<Uuid>,
        driver_label -> Nullable<Text>,
        evidence -> Nullable<Jsonb>,
        impact_cents -> Nullable<Int8>,
        impact_currency -> Nullable<Text>,
    }
}

diesel::table! {
    alert_segment_member (alert_definition_id, customer_id) {
        alert_definition_id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        entered_at -> Timestamp,
        left_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    analytics_dashboard_share (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        saved_view_id -> Uuid,
        token -> Text,
        snapshot -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    analytics_projection_quarantine (stream, source_id) {
        stream -> Text,
        source_id -> Uuid,
        tenant_id -> Uuid,
        reason -> Text,
        first_seen_at -> Timestamptz,
        last_tried_at -> Timestamptz,
        resolved_at -> Nullable<Timestamptz>,
        resume_after -> Nullable<Uuid>,
    }
}

diesel::table! {
    analytics_saved_view (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        name -> Text,
        metric_key -> Text,
        visualization -> Text,
        config -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    api_idempotency_key (tenant_id, operation, idempotency_key) {
        tenant_id -> Uuid,
        operation -> Text,
        idempotency_key -> Text,
        request_fingerprint -> Bytea,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    api_token (id) {
        id -> Uuid,
        name -> Text,
        created_at -> Timestamp,
        tenant_id -> Uuid,
        hash -> Text,
        hint -> Text,
        first_used_at -> Nullable<Timestamp>,
        last_used_at -> Nullable<Timestamp>,
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
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BatchJobTypeEnum;
    use super::sql_types::BatchJobStatusEnum;

    batch_job (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        job_type -> BatchJobTypeEnum,
        status -> BatchJobStatusEnum,
        input_source_key -> Nullable<Text>,
        input_params -> Nullable<Jsonb>,
        total_items -> Nullable<Int4>,
        processed_items -> Int4,
        failed_items -> Int4,
        file_hash -> Nullable<Text>,
        locked_at -> Nullable<Timestamp>,
        created_by -> Uuid,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        completed_at -> Nullable<Timestamp>,
        error_message -> Nullable<Text>,
        error_output_key -> Nullable<Text>,
        input_file_name -> Nullable<Text>,
        output_key -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BatchJobChunkStatusEnum;

    batch_job_chunk (id) {
        id -> Uuid,
        job_id -> Uuid,
        tenant_id -> Uuid,
        chunk_index -> Int4,
        status -> BatchJobChunkStatusEnum,
        item_offset -> Int4,
        item_count -> Int4,
        processed_count -> Int4,
        failed_count -> Int4,
        retry_count -> Int4,
        max_retries -> Int4,
        locked_at -> Nullable<Timestamp>,
        retry_after -> Nullable<Timestamp>,
        events -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    batch_job_entity (id) {
        id -> Uuid,
        batch_job_id -> Uuid,
        tenant_id -> Uuid,
        entity_type -> Text,
        entity_id -> Uuid,
        created_at -> Timestamp,
    }
}

diesel::table! {
    batch_job_item_failure (id) {
        id -> Uuid,
        chunk_id -> Uuid,
        job_id -> Uuid,
        tenant_id -> Uuid,
        item_index -> Int4,
        item_identifier -> Nullable<Text>,
        reason -> Text,
        created_at -> Timestamp,
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
        invoice_id -> Nullable<Uuid>,
        credit_note_id -> Nullable<Uuid>,
        plan_version_id -> Uuid,
        tenant_id -> Uuid,
        subscription_id -> Uuid,
        customer_id -> Uuid,
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
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        tenant_id -> Uuid,
        product_family_id -> Uuid,
        product_id -> Nullable<Uuid>,
        filters -> Nullable<Jsonb>,
        unit_singular -> Nullable<Text>,
        unit_plural -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CheckoutSessionStatusEnum;
    use super::sql_types::CheckoutTypeEnum;

    checkout_session (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        plan_version_id -> Nullable<Uuid>,
        billing_start_date -> Nullable<Date>,
        billing_day_anchor -> Nullable<Int2>,
        net_terms -> Nullable<Int4>,
        trial_duration_days -> Nullable<Int4>,
        end_date -> Nullable<Date>,
        auto_advance_invoices -> Bool,
        charge_automatically -> Bool,
        invoice_memo -> Nullable<Text>,
        invoice_threshold -> Nullable<Numeric>,
        purchase_order -> Nullable<Text>,
        components -> Nullable<Jsonb>,
        add_ons -> Nullable<Jsonb>,
        coupon_code -> Nullable<Varchar>,
        coupon_ids -> Array<Nullable<Uuid>>,
        status -> CheckoutSessionStatusEnum,
        created_at -> Timestamptz,
        expires_at -> Nullable<Timestamptz>,
        completed_at -> Nullable<Timestamptz>,
        subscription_id -> Nullable<Uuid>,
        metadata -> Nullable<Jsonb>,
        checkout_type -> CheckoutTypeEnum,
        payment_methods_config -> Nullable<Jsonb>,
        change_date -> Nullable<Date>,
        success_url -> Nullable<Text>,
        cancel_url -> Nullable<Text>,
        pre_payment_coupon_code -> Nullable<Varchar>,
        pre_payment_coupon_ids -> Nullable<Array<Nullable<Uuid>>>,
    }
}

diesel::table! {
    connect_access_token (id) {
        id -> Uuid,
        token_hash -> Text,
        oauth_app_id -> Uuid,
        connected_organization_id -> Uuid,
        scopes -> Array<Nullable<Text>>,
        expires_at -> Timestamptz,
        created_at -> Timestamptz,
        revoked_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    connect_authorization_code (id) {
        id -> Uuid,
        code_hash -> Text,
        oauth_app_id -> Uuid,
        connected_organization_id -> Uuid,
        redirect_uri -> Text,
        scopes -> Array<Nullable<Text>>,
        code_challenge -> Nullable<Text>,
        code_challenge_method -> Nullable<Text>,
        expires_at -> Timestamptz,
        created_at -> Timestamptz,
        used_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ConnectConnectionTypeEnum;
    use super::sql_types::ConnectConnectionStatusEnum;
    use super::sql_types::ConnectOnboardingModeEnum;

    connect_connected_account (id) {
        id -> Uuid,
        platform_organization_id -> Uuid,
        connected_organization_id -> Nullable<Uuid>,
        connection_type -> ConnectConnectionTypeEnum,
        status -> ConnectConnectionStatusEnum,
        onboarding_mode -> ConnectOnboardingModeEnum,
        onboarding_completed_at -> Nullable<Timestamptz>,
        metadata -> Nullable<Jsonb>,
        created_at -> Timestamptz,
        revoked_at -> Nullable<Timestamptz>,
        pending_email -> Nullable<Text>,
        pending_organization_name -> Nullable<Text>,
        pending_country -> Nullable<Text>,
        platform_customer_id -> Nullable<Uuid>,
        connected_tenant_id -> Nullable<Uuid>,
        granted_scopes -> Array<Nullable<Text>>,
    }
}

diesel::table! {
    connect_oauth_app (id) {
        id -> Uuid,
        organization_id -> Uuid,
        name -> Text,
        client_id -> Text,
        client_secret_hash -> Text,
        client_secret_hint -> Text,
        redirect_uris -> Array<Nullable<Text>>,
        scopes -> Array<Nullable<Text>>,
        is_active -> Bool,
        created_at -> Timestamptz,
        updated_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    connect_onboarding_link (id) {
        id -> Uuid,
        connected_account_id -> Uuid,
        token_hash -> Text,
        redirect_url -> Text,
        expires_at -> Timestamptz,
        used_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    connect_platform_settings (organization_id) {
        organization_id -> Uuid,
        is_platform_enabled -> Bool,
        default_scopes -> Array<Nullable<Text>>,
        branding_invoicing_entity_id -> Nullable<Uuid>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    connect_refresh_token (id) {
        id -> Uuid,
        token_hash -> Text,
        access_token_id -> Uuid,
        expires_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
        revoked_at -> Nullable<Timestamptz>,
        rotated_to_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ConnectorTypeEnum;
    use super::sql_types::ConnectorProviderEnum;
    use super::sql_types::ConnectorStatusEnum;
    use super::sql_types::ConnectorWebhookRouteEnum;

    connector (id) {
        id -> Uuid,
        created_at -> Timestamp,
        tenant_id -> Uuid,
        alias -> Text,
        connector_type -> ConnectorTypeEnum,
        provider -> ConnectorProviderEnum,
        data -> Nullable<Jsonb>,
        sensitive -> Nullable<Text>,
        status -> ConnectorStatusEnum,
        updated_at -> Timestamp,
        account -> Nullable<Jsonb>,
        webhook_route -> Nullable<ConnectorWebhookRouteEnum>,
        credentials_error -> Nullable<Text>,
        credentials_error_at -> Nullable<Timestamp>,
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
    coupon_plan (coupon_id, plan_id) {
        coupon_id -> Uuid,
        plan_id -> Uuid,
    }
}

diesel::table! {
    credit_balance_projection (account_id) {
        account_id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        computed_at -> Timestamptz,
        usage_as_of -> Timestamptz,
        pending_minor -> Int8,
        live_available_minor -> Int8,
        overage_minor -> Int8,
        per_grant -> Jsonb,
        metric_ids -> Array<Nullable<Uuid>>,
        per_subscription -> Jsonb,
        charged -> Nullable<Jsonb>,
        pending_by_product -> Jsonb,
        ledger_version -> Int8,
    }
}

diesel::table! {
    credit_customer_inputs (tenant_id, customer_id) {
        tenant_id -> Uuid,
        customer_id -> Uuid,
        inputs_token -> Int8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Xid8;
    use super::sql_types::CreditInputChangeKindEnum;

    credit_input_change (shard, store_xid, id, created_on) {
        id -> Int8,
        store_xid -> Xid8,
        shard -> Int2,
        tenant_id -> Uuid,
        customer_id -> Nullable<Uuid>,
        kind -> CreditInputChangeKindEnum,
        created_on -> Date,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Xid8;

    credit_input_change_retention (partition_day) {
        partition_day -> Date,
        max_store_xid -> Nullable<Xid8>,
        max_id -> Nullable<Int8>,
        dropped_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CreditNoteStatus;
    use super::sql_types::CreditTypeEnum;
    use super::sql_types::EInvoicingStatusEnum;
    use super::sql_types::Xid8;

    credit_note (id) {
        id -> Uuid,
        credit_note_number -> Text,
        status -> CreditNoteStatus,
        created_at -> Timestamptz,
        updated_at -> Nullable<Timestamptz>,
        finalized_at -> Nullable<Timestamptz>,
        voided_at -> Nullable<Timestamptz>,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        invoice_id -> Uuid,
        invoice_number -> Text,
        plan_version_id -> Nullable<Uuid>,
        subscription_id -> Nullable<Uuid>,
        currency -> Text,
        subtotal -> Int8,
        tax_amount -> Int8,
        total -> Int8,
        refunded_amount_cents -> Int8,
        credited_amount_cents -> Int8,
        line_items -> Jsonb,
        tax_breakdown -> Jsonb,
        reason -> Nullable<Text>,
        memo -> Nullable<Text>,
        customer_details -> Jsonb,
        seller_details -> Jsonb,
        pdf_document_id -> Nullable<Uuid>,
        conn_meta -> Nullable<Jsonb>,
        invoicing_entity_id -> Uuid,
        credit_type -> CreditTypeEnum,
        store_committed_at -> Timestamptz,
        custom_properties -> Jsonb,
        einvoicing_status -> Nullable<EInvoicingStatusEnum>,
        einvoicing_report -> Nullable<Jsonb>,
        xml_document_id -> Nullable<Uuid>,
        debt_cancelled_amount_cents -> Int8,
        credited_retired_balance -> Bool,
        voids_invoice -> Bool,
        store_xid -> Nullable<Xid8>,
    }
}

diesel::table! {
    credit_recompute_mark (tenant_id, customer_id) {
        tenant_id -> Uuid,
        customer_id -> Uuid,
        marked_at -> Timestamptz,
    }
}

diesel::table! {
    credit_tenant_inputs (tenant_id) {
        tenant_id -> Uuid,
        inputs_token -> Int8,
    }
}

diesel::table! {
    credit_unit (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        code -> Text,
        name -> Text,
        plural_name -> Text,
        symbol -> Nullable<Text>,
        precision -> Int2,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        archived_at -> Nullable<Timestamptz>,
        product_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    credit_unit_price (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        credit_unit_id -> Uuid,
        currency -> Text,
        unit_price -> Numeric,
        created_at -> Timestamptz,
        archived_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    credit_usage_settlement (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        account_id -> Uuid,
        subscription_id -> Uuid,
        period_start -> Date,
        event_time_bound -> Timestamptz,
        amount -> Int8,
        tax_inclusive_rate -> Nullable<Numeric>,
        give_back_source_id -> Uuid,
        invoice_id -> Nullable<Uuid>,
        created_at -> Timestamptz,
        returned_at -> Nullable<Timestamptz>,
        tax_minor -> Int8,
        ordinal -> Int4,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CustomPropertyEntityTypeEnum;
    use super::sql_types::CustomPropertyTypeEnum;

    custom_property_definition (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        entity_type -> CustomPropertyEntityTypeEnum,
        key -> Text,
        name -> Text,
        description -> Nullable<Text>,
        property_type -> CustomPropertyTypeEnum,
        config -> Jsonb,
        required -> Bool,
        default_value -> Nullable<Jsonb>,
        display_order -> Int4,
        archived_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    custom_tax (id) {
        id -> Uuid,
        invoicing_entity_id -> Uuid,
        name -> Text,
        tax_code -> Text,
        rules -> Jsonb,
        tax_category_id -> Nullable<Uuid>,
        tenant_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CustomerVatValidationStatusEnum;
    use super::sql_types::CustomerTypeEnum;
    use super::sql_types::CustomerTaxStatusEnum;

    customer (id) {
        id -> Uuid,
        name -> Text,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
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
        current_payment_method_id -> Nullable<Uuid>,
        vat_number -> Nullable<Text>,
        invoicing_emails -> Array<Nullable<Text>>,
        conn_meta -> Nullable<Jsonb>,
        vat_number_format_valid -> Bool,
        custom_taxes -> Jsonb,
        connected_account_id -> Nullable<Uuid>,
        vat_number_validation_status -> Nullable<CustomerVatValidationStatusEnum>,
        vat_number_checked_at -> Nullable<Timestamp>,
        vat_number_vies_check -> Nullable<Jsonb>,
        store_committed_at -> Timestamptz,
        custom_properties -> Jsonb,
        customer_type -> CustomerTypeEnum,
        first_name -> Nullable<Text>,
        last_name -> Nullable<Text>,
        tax_status -> CustomerTaxStatusEnum,
        exemption_reason -> Nullable<Text>,
        legal_number -> Nullable<Text>,
        preferred_locales -> Array<Nullable<Text>>,
        owner_user_id -> Nullable<Uuid>,
        payment_reminders_opt_out -> Bool,
        payment_reminder_sequence_id -> Nullable<Uuid>,
        buyer_reference -> Nullable<Text>,
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
        currency -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BudgetBehaviorEnum;
    use super::sql_types::BudgetAuthorEnum;

    customer_budget (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        currency -> Text,
        amount_minor -> Int8,
        thresholds -> Array<Nullable<Int4>>,
        behavior -> BudgetBehaviorEnum,
        created_by -> BudgetAuthorEnum,
        version -> Int4,
        archived_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        credit_unit_id -> Nullable<Uuid>,
        subscription_id -> Nullable<Uuid>,
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
    use super::sql_types::UsageAtZeroEnum;

    customer_credit_account (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        currency -> Text,
        created_at -> Timestamptz,
        low_balance_thresholds_minor -> Array<Nullable<Int8>>,
        credit_unit_id -> Nullable<Uuid>,
        unit_price -> Nullable<Numeric>,
        unit_price_currency -> Nullable<Text>,
        ledger_version -> Int8,
        usage_at_zero -> UsageAtZeroEnum,
    }
}

diesel::table! {
    customer_credit_auto_top_up (account_id) {
        account_id -> Uuid,
        tenant_id -> Uuid,
        method -> Text,
        threshold_minor -> Int8,
        amount_minor -> Nullable<Int8>,
        target_minor -> Nullable<Int8>,
        minimum_minor -> Nullable<Int8>,
        maximum_minor -> Nullable<Int8>,
        daily_cap_minor -> Nullable<Int8>,
        monthly_cap_minor -> Nullable<Int8>,
        payment_method_id -> Nullable<Uuid>,
        parked_at -> Nullable<Timestamptz>,
        consecutive_failures -> Int4,
        last_attempted_at -> Nullable<Timestamptz>,
        last_succeeded_at -> Nullable<Timestamptz>,
        suspended_at -> Nullable<Timestamptz>,
        last_error -> Nullable<Text>,
        retry_due_at -> Nullable<Timestamptz>,
        authorization_revision -> Nullable<Text>,
        authorization_actor -> Nullable<Text>,
        authorization_recorded_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        agreed_price_id -> Nullable<Uuid>,
        agreed_unit_price -> Nullable<Numeric>,
        agreed_currency -> Nullable<Text>,
        daily_cap_is_default -> Bool,
        maximum_is_default -> Bool,
        consent_attested_by -> Nullable<Text>,
        consent_attested_at -> Nullable<Timestamptz>,
        consent_attested_revision -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Xid8;

    customer_credit_entry (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        grant_id -> Uuid,
        kind -> Text,
        amount -> Int8,
        source_id -> Uuid,
        source_kind -> Nullable<Text>,
        invoice_id -> Nullable<Uuid>,
        credit_note_id -> Nullable<Uuid>,
        created_at -> Timestamptz,
        occurred_at -> Timestamptz,
        store_committed_at -> Timestamptz,
        store_xid -> Xid8,
        reason -> Nullable<Text>,
        created_by -> Nullable<Uuid>,
        cost_minor -> Nullable<Int8>,
        payment_movement_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    customer_credit_grant (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        account_id -> Uuid,
        amount -> Int8,
        remaining -> Int8,
        priority -> Int4,
        classification -> Text,
        effective_at -> Timestamptz,
        expires_at -> Nullable<Timestamptz>,
        source_kind -> Text,
        source_id -> Uuid,
        invoice_id -> Nullable<Uuid>,
        line_local_id -> Nullable<Text>,
        credit_note_id -> Nullable<Uuid>,
        reason -> Text,
        reference -> Nullable<Text>,
        created_by -> Nullable<Uuid>,
        created_at -> Timestamptz,
        subscription_id -> Nullable<Uuid>,
        application -> Text,
        ends_at -> Nullable<Timestamptz>,
        end_reason -> Nullable<Text>,
        lapses_at -> Nullable<Timestamptz>,
        cost_minor -> Nullable<Int8>,
        cost_currency -> Nullable<Text>,
        rollover -> Nullable<Jsonb>,
        pays_usage_from -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    customer_credit_grant_product (grant_id, product_id) {
        tenant_id -> Uuid,
        grant_id -> Uuid,
        product_id -> Uuid,
    }
}

diesel::table! {
    customer_credit_line_allocation (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        invoice_id -> Uuid,
        credit_note_id -> Nullable<Uuid>,
        grant_id -> Uuid,
        line_local_id -> Nullable<Text>,
        amount -> Int8,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    customer_credit_operation (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        account_id -> Uuid,
        state -> Text,
        namespace -> Text,
        idempotency_key -> Text,
        request_fingerprint -> Bytea,
        amount_minor -> Int8,
        expected_consideration_minor -> Nullable<Int8>,
        consideration_currency -> Nullable<Text>,
        payment_transaction_id -> Nullable<Uuid>,
        invoice_id -> Nullable<Uuid>,
        reason -> Text,
        metadata -> Jsonb,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        settled_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    customer_credit_purchase (id) {
        id -> Uuid,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        amount_cents -> Int8,
        note -> Nullable<Text>,
        expires_at -> Nullable<Timestamptz>,
        invoice_id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        created_by -> Nullable<Uuid>,
        amount_credits -> Nullable<Int8>,
        negotiated_unit_price -> Nullable<Numeric>,
        unit_price_source -> Nullable<Text>,
    }
}

diesel::table! {
    customer_credit_unrecovered (tenant_id, account_id, source_id) {
        tenant_id -> Uuid,
        account_id -> Uuid,
        invoice_id -> Nullable<Uuid>,
        payment_id -> Nullable<Uuid>,
        source_id -> Uuid,
        reported -> Int8,
        notices -> Int4,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CustomerMilestoneEnum;

    customer_milestone (tenant_id, customer_id, milestone) {
        tenant_id -> Uuid,
        customer_id -> Uuid,
        milestone -> CustomerMilestoneEnum,
        occurred_at -> Timestamp,
        created_at -> Timestamp,
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
        fingerprint -> Nullable<Text>,
        card_refreshed_at -> Nullable<Timestamptz>,
        saved_for_payment_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::DeadLetterStatusEnum;

    dead_letter_message (id) {
        id -> Uuid,
        tenant_id -> Nullable<Uuid>,
        queue -> Text,
        pgmq_msg_id -> Int8,
        message -> Nullable<Jsonb>,
        headers -> Nullable<Jsonb>,
        read_ct -> Int4,
        enqueued_at -> Timestamptz,
        dead_lettered_at -> Timestamptz,
        last_error -> Nullable<Text>,
        status -> DeadLetterStatusEnum,
        resolved_at -> Nullable<Timestamptz>,
        resolved_by -> Nullable<Uuid>,
        requeued_pgmq_msg_id -> Nullable<Int8>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    dogfooding_sync_state (stream) {
        stream -> Text,
        cursor_at -> Timestamp,
        cursor_id -> Uuid,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::EmailKindAudienceModeEnum;

    email_kind_audience (invoicing_entity_id, kind) {
        invoicing_entity_id -> Uuid,
        tenant_id -> Uuid,
        kind -> Text,
        mode -> EmailKindAudienceModeEnum,
        audience -> Jsonb,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::EmailSendClaimStateEnum;

    email_send_claim (dedupe_key) {
        dedupe_key -> Text,
        tenant_id -> Uuid,
        state -> EmailSendClaimStateEnum,
        claimed_at -> Timestamp,
        finished_at -> Nullable<Timestamp>,
        sent_email_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::EntitlementEntityTypeEnum;
    use super::sql_types::EntitlementModeEnum;

    entitlement (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        feature_id -> Uuid,
        entity_id -> Uuid,
        entity_type -> EntitlementEntityTypeEnum,
        mode -> EntitlementModeEnum,
        value -> Jsonb,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ActorTypeEnum;

    entity_activity (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        entity_type -> Text,
        entity_id -> Uuid,
        activity_type -> Text,
        actor_type -> ActorTypeEnum,
        metadata -> Nullable<Jsonb>,
        occurred_at -> Timestamptz,
        agg_customer_id -> Nullable<Uuid>,
        agg_subscription_id -> Nullable<Uuid>,
        actor_uuid -> Nullable<Uuid>,
        actor_alias -> Nullable<Text>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::EntityListTypeEnum;

    entity_list_view (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        entity_type -> EntityListTypeEnum,
        name -> Text,
        config -> Jsonb,
        shared -> Bool,
        created_by -> Nullable<Uuid>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::EntityListTypeEnum;

    entity_list_view_default (tenant_id, user_id, entity_type) {
        tenant_id -> Uuid,
        user_id -> Uuid,
        entity_type -> EntityListTypeEnum,
        view_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::FeatureTypeEnum;
    use super::sql_types::FeatureStatusEnum;
    use super::sql_types::ConfigValueTypeEnum;

    feature (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        product_id -> Nullable<Uuid>,
        name -> Text,
        description -> Nullable<Text>,
        feature_type -> FeatureTypeEnum,
        status -> FeatureStatusEnum,
        metric_id -> Nullable<Uuid>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        code -> Text,
        config_options -> Nullable<Array<Nullable<Text>>>,
        config_value_type -> Nullable<ConfigValueTypeEnum>,
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
    instance_telemetry (id) {
        id -> Bool,
        instance_id -> Uuid,
        installed_at -> Timestamp,
        first_version -> Text,
        last_sent_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    instance_license (id) {
        id -> Bool,
        instance_id -> Uuid,
        signing_key -> Text,
        refresh_token -> Nullable<Text>,
        license_token -> Nullable<Text>,
        refreshed_at -> Nullable<Timestamp>,
        last_error -> Nullable<Text>,
        purchase_started_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        next_refresh_at -> Nullable<Timestamp>,
        refresh_failures -> Int4,
        revoked_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::InvoiceStatusEnum;
    use super::sql_types::InvoiceType;
    use super::sql_types::InvoicePaymentStatus;
    use super::sql_types::EInvoicingStatusEnum;
    use super::sql_types::Xid8;

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
        coupons -> Jsonb,
        tax_breakdown -> Jsonb,
        manual -> Bool,
        voided_at -> Nullable<Timestamp>,
        marked_as_uncollectible_at -> Nullable<Timestamp>,
        invoicing_entity_id -> Uuid,
        parent_invoice_id -> Nullable<Uuid>,
        consolidated_into_invoice_id -> Nullable<Uuid>,
        annexes -> Jsonb,
        store_committed_at -> Timestamptz,
        custom_properties -> Jsonb,
        cycle_index -> Nullable<Int4>,
        manually_edited_at -> Nullable<Timestamp>,
        customer_details_overridden_at -> Nullable<Timestamp>,
        billing_period_start -> Date,
        einvoicing_status -> Nullable<EInvoicingStatusEnum>,
        einvoicing_report -> Nullable<Jsonb>,
        tax_inclusive -> Bool,
        auto_charge_requested_at -> Nullable<Timestamp>,
        promotional_credits -> Int8,
        store_xid -> Nullable<Xid8>,
        payment_disputed_at -> Nullable<Timestamp>,
        delivery_held_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::InvoiceAnnexKindEnum;

    invoice_annex_template (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        name -> Text,
        kind -> InvoiceAnnexKindEnum,
        definition -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ReminderStateEnum;
    use super::sql_types::ReminderNotEnrolledReasonEnum;
    use super::sql_types::ReminderEnrolledViaEnum;
    use super::sql_types::ReminderStopReasonEnum;

    invoice_payment_reminder (invoice_id) {
        invoice_id -> Uuid,
        tenant_id -> Uuid,
        state -> ReminderStateEnum,
        not_enrolled_reason -> Nullable<ReminderNotEnrolledReasonEnum>,
        resolved_at -> Timestamp,
        generation -> Int4,
        sequence_id -> Nullable<Uuid>,
        enrolled_via -> Nullable<ReminderEnrolledViaEnum>,
        enrolled_at -> Nullable<Timestamp>,
        last_offset_days -> Nullable<Int4>,
        next_reminder_at -> Nullable<Timestamp>,
        paused_until -> Nullable<Timestamp>,
        stopped_at -> Nullable<Timestamp>,
        stop_reason -> Nullable<ReminderStopReasonEnum>,
        completed_at -> Nullable<Timestamp>,
        ended_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::TaxResolverEnum;
    use super::sql_types::EInvoicingFormatEnum;

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
        consolidate_recurring_invoices -> Bool,
        portal_theme_mode -> Nullable<Text>,
        portal_roundness -> Nullable<Text>,
        require_vies_valid_for_reverse_charge -> Bool,
        invoicing_language -> Nullable<Text>,
        invoice_footer_translations -> Jsonb,
        default_tax_category_id -> Nullable<Uuid>,
        tax_provider_id -> Nullable<Uuid>,
        einvoicing_format -> Nullable<EInvoicingFormatEnum>,
        legal_number -> Nullable<Text>,
        credit_note_number_pattern -> Text,
        customer_locale -> Nullable<Text>,
        customer_extra_locales -> Array<Nullable<Text>>,
        brand_color_light -> Nullable<Text>,
        contact_name -> Nullable<Text>,
        contact_phone -> Nullable<Text>,
        contact_email -> Nullable<Text>,
        portal_credit_top_up_enabled -> Bool,
        portal_auto_top_up_enabled -> Bool,
        portal_budgets_enabled -> Bool,
        require_billing_information -> Bool,
    }
}

diesel::table! {
    invoicing_entity_email_settings (invoicing_entity_id) {
        invoicing_entity_id -> Uuid,
        tenant_id -> Uuid,
        sender_name -> Nullable<Text>,
        reply_to -> Nullable<Text>,
        bcc -> Array<Nullable<Text>>,
        disabled_kinds -> Array<Nullable<Text>>,
        timezone -> Nullable<Text>,
        reminder_send_hour -> Int2,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    issued_license (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        subscription_id -> Nullable<Uuid>,
        auto_issued -> Bool,
        status -> Text,
        instance_id -> Nullable<Text>,
        instance_public_key -> Nullable<Text>,
        activation_code_encrypted -> Nullable<Text>,
        token_hash -> Text,
        previous_token_hash -> Nullable<Text>,
        previous_token_valid_until -> Nullable<Timestamp>,
        validity_days -> Int4,
        last_refreshed_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
        revoked_at -> Nullable<Timestamp>,
        revoked_immediately -> Bool,
        offline -> Bool,
        token_generation -> Int4,
        holder_session -> Nullable<Text>,
        contender_session -> Nullable<Text>,
        contender_since -> Nullable<Timestamp>,
        contender_seen_at -> Nullable<Timestamp>,
        contender_url -> Nullable<Text>,
        stats_reported_through -> Nullable<Date>,
    }
}

diesel::table! {
    license_signing_key (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        kid -> Text,
        public_key_pem -> Text,
        private_key_encrypted -> Nullable<Text>,
        created_at -> Timestamp,
        retired_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    licensing_endpoint (tenant_id) {
        tenant_id -> Uuid,
        slug -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::MetricTargetPeriodGrainEnum;

    metric_target (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        metric_key -> Text,
        cohort -> Nullable<Jsonb>,
        scope_label -> Nullable<Text>,
        owner_user_id -> Nullable<Uuid>,
        period_grain -> MetricTargetPeriodGrainEnum,
        period_start -> Date,
        target_value -> Numeric,
        currency -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        final_value -> Nullable<Float8>,
        finalized_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::MetricTargetChangeKindEnum;

    metric_target_change (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        metric_target_id -> Uuid,
        kind -> MetricTargetChangeKindEnum,
        imported -> Bool,
        actor_user_id -> Nullable<Uuid>,
        before -> Nullable<Jsonb>,
        after -> Nullable<Jsonb>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    metric_target_snapshot (metric_target_id, snapshot_date) {
        metric_target_id -> Uuid,
        snapshot_date -> Date,
        tenant_id -> Uuid,
        actual -> Nullable<Float8>,
        projected -> Nullable<Float8>,
        expected -> Nullable<Float8>,
        status -> Text,
        scheduled_count -> Int4,
        reconstructed -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ChannelTypeEnum;

    notification_channel (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        name -> Text,
        channel_type -> ChannelTypeEnum,
        config -> Jsonb,
        disabled -> Bool,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
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
    use diesel::sql_types::*;
    use super::sql_types::ActorTypeEnum;

    org_security_audit_log (id) {
        id -> Uuid,
        organization_id -> Uuid,
        event_type -> Text,
        actor_type -> ActorTypeEnum,
        actor_id -> Nullable<Uuid>,
        actor_email -> Nullable<Text>,
        target_user_id -> Nullable<Uuid>,
        target_email -> Nullable<Text>,
        metadata -> Nullable<Jsonb>,
        ip_address -> Nullable<Text>,
        user_agent -> Nullable<Text>,
        ua_browser -> Nullable<Text>,
        ua_os -> Nullable<Text>,
        ua_device -> Nullable<Text>,
        occurred_at -> Timestamptz,
    }
}

diesel::table! {
    organization (id) {
        id -> Uuid,
        trade_name -> Text,
        slug -> Text,
        created_at -> Timestamp,
        archived_at -> Nullable<Timestamp>,
        default_country -> Text,
        is_express -> Bool,
        mfa_required -> Bool,
    }
}

diesel::table! {
    organization_domain (id) {
        id -> Uuid,
        organization_id -> Uuid,
        domain -> Text,
        verification_token -> Text,
        verified_at -> Nullable<Timestamptz>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::OrganizationUserRole;

    organization_invite (id) {
        id -> Uuid,
        organization_id -> Uuid,
        invited_email -> Text,
        invited_by -> Uuid,
        role -> OrganizationUserRole,
        created_at -> Timestamptz,
        expires_at -> Timestamptz,
        accepted_at -> Nullable<Timestamptz>,
        revoked_at -> Nullable<Timestamptz>,
        role_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::OrganizationUserRole;

    organization_member (user_id, organization_id) {
        user_id -> Uuid,
        organization_id -> Uuid,
        role -> OrganizationUserRole,
        role_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::MilestoneEnum;

    organization_milestone (organization_id, milestone) {
        organization_id -> Uuid,
        milestone -> MilestoneEnum,
        occurred_at -> Timestamp,
        tenant_id -> Nullable<Uuid>,
        created_at -> Timestamp,
        id -> Uuid,
        production_occurred_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    organization_sso_connection (id) {
        id -> Uuid,
        organization_id -> Uuid,
        provider -> Text,
        domains -> Array<Nullable<Text>>,
        issuer_url -> Text,
        authorize_url -> Text,
        token_url -> Text,
        userinfo_url -> Text,
        client_id -> Text,
        client_secret -> Text,
        scopes -> Text,
        enabled -> Bool,
        enforced -> Bool,
        jit_provisioning -> Bool,
        created_at -> Timestamptz,
        updated_at -> Nullable<Timestamptz>,
        created_by -> Nullable<Uuid>,
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
    payment_reminder_sequence (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        invoicing_entity_id -> Uuid,
        name -> Text,
        position -> Int4,
        enabled -> Bool,
        audience -> Nullable<Jsonb>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::PaymentReminderStepActionEnum;

    payment_reminder_step (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        sequence_id -> Uuid,
        offset_days -> Int4,
        action -> PaymentReminderStepActionEnum,
        content -> Jsonb,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ReversalKindEnum;
    use super::sql_types::Xid8;

    payment_reversal_movement (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        payment_transaction_id -> Uuid,
        invoice_id -> Nullable<Uuid>,
        customer_id -> Uuid,
        disputed -> Bool,
        amount -> Int8,
        excess -> Int8,
        reversal_kind -> Nullable<ReversalKindEnum>,
        provider_ref -> Nullable<Text>,
        occurred_at -> Timestamp,
        created_at -> Timestamp,
        store_xid -> Xid8,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::PaymentStatusEnum;
    use super::sql_types::PaymentTypeEnum;
    use super::sql_types::ReversalKindEnum;
    use super::sql_types::RefundModeEnum;
    use super::sql_types::DeclineKindEnum;

    payment_transaction (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        invoice_id -> Nullable<Uuid>,
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
        checkout_session_id -> Nullable<Uuid>,
        pending_plan_version_id -> Nullable<Uuid>,
        store_committed_at -> Timestamptz,
        initiated_by_customer_id -> Nullable<Uuid>,
        created_at -> Timestamptz,
        next_action -> Nullable<Jsonb>,
        amount_refunded -> Int8,
        pending_provider_intent_id -> Nullable<Text>,
        pending_connection_id -> Nullable<Uuid>,
        amount_reversed -> Int8,
        reversal_kind -> Nullable<ReversalKindEnum>,
        reversal_reason -> Nullable<Text>,
        parent_transaction_id -> Nullable<Uuid>,
        credit_note_id -> Nullable<Uuid>,
        refund_mode -> Nullable<RefundModeEnum>,
        last_reconciled_at -> Nullable<Timestamptz>,
        decline_kind -> Nullable<DeclineKindEnum>,
        amount_disputed -> Int8,
        amount_reversed_excess -> Int8,
        refund_excess -> Int8,
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
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        tenant_id -> Uuid,
        product_family_id -> Uuid,
        plan_type -> PlanTypeEnum,
        status -> PlanStatusEnum,
        active_version_id -> Nullable<Uuid>,
        draft_version_id -> Nullable<Uuid>,
        self_service_rank -> Nullable<Int4>,
        store_committed_at -> Timestamptz,
        issues_license -> Bool,
    }
}

diesel::table! {
    plan_component_price (plan_component_id, price_id) {
        plan_component_id -> Uuid,
        price_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::CreditExhaustedRuleEnum;

    plan_version (id) {
        id -> Uuid,
        is_draft_version -> Bool,
        plan_id -> Uuid,
        version -> Int4,
        trial_duration_days -> Nullable<Int4>,
        tenant_id -> Uuid,
        period_start_day -> Nullable<Int2>,
        net_terms -> Int4,
        currency -> Text,
        billing_cycles -> Nullable<Int4>,
        created_at -> Timestamp,
        trialing_plan_id -> Nullable<Uuid>,
        trial_is_free -> Bool,
        uses_product_pricing -> Bool,
        tax_inclusive -> Bool,
        credit_exhausted_rule -> Nullable<CreditExhaustedRuleEnum>,
        credit_exhausted_caps -> Jsonb,
    }
}

diesel::table! {
    plan_version_add_on (id) {
        id -> Uuid,
        plan_version_id -> Uuid,
        add_on_id -> Uuid,
        price_id -> Nullable<Uuid>,
        self_serviceable -> Nullable<Bool>,
        max_instances_per_subscription -> Nullable<Int4>,
        tenant_id -> Uuid,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::AdjustmentKindEnum;

    plan_version_adjustment (plan_version_id, kind) {
        plan_version_id -> Uuid,
        kind -> AdjustmentKindEnum,
        amount_subunit -> Int8,
        includes_all_components -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::AdjustmentKindEnum;

    plan_version_adjustment_component (plan_version_id, kind, product_id) {
        plan_version_id -> Uuid,
        kind -> AdjustmentKindEnum,
        product_id -> Uuid,
    }
}

diesel::table! {
    plan_version_annex_template (plan_version_id, annex_template_id) {
        plan_version_id -> Uuid,
        annex_template_id -> Uuid,
        tenant_id -> Uuid,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BillingPeriodEnum;

    price (id) {
        id -> Uuid,
        product_id -> Uuid,
        cadence -> BillingPeriodEnum,
        currency -> Text,
        pricing -> Jsonb,
        tenant_id -> Uuid,
        created_at -> Timestamp,
        archived_at -> Nullable<Timestamp>,
        catalog -> Bool,
        credit_unit_id -> Nullable<Uuid>,
        credit_unit_rate -> Nullable<Numeric>,
    }
}

diesel::table! {
    price_component (id) {
        id -> Uuid,
        name -> Text,
        legacy_fee -> Nullable<Jsonb>,
        plan_version_id -> Uuid,
        product_id -> Nullable<Uuid>,
        billable_metric_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::FeeTypeEnum;

    product (id) {
        id -> Uuid,
        name -> Text,
        description -> Nullable<Text>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        tenant_id -> Uuid,
        product_family_id -> Uuid,
        fee_type -> FeeTypeEnum,
        fee_structure -> Jsonb,
        catalog -> Bool,
        store_committed_at -> Timestamptz,
        tax_category_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    product_accounting (product_id, invoicing_entity_id) {
        product_id -> Uuid,
        invoicing_entity_id -> Uuid,
        product_code -> Nullable<Text>,
        ledger_account_code -> Nullable<Text>,
    }
}

diesel::table! {
    product_custom_tax (product_id, invoicing_entity_id, custom_tax_id) {
        product_id -> Uuid,
        invoicing_entity_id -> Uuid,
        custom_tax_id -> Uuid,
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
    promo_code_attempt (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        attempted_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::TaxIdCollectionEnum;
    use super::sql_types::CustomerTypeEnum;

    purchase_link (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        plan_id -> Uuid,
        name -> Text,
        token -> Text,
        allow_coupon_codes -> Bool,
        listed -> Bool,
        success_url -> Nullable<Text>,
        cancel_url -> Nullable<Text>,
        require_billing_address -> Bool,
        tax_id_collection -> TaxIdCollectionEnum,
        only_buyer_type -> Nullable<CustomerTypeEnum>,
        created_at -> Timestamp,
        updated_at -> Timestamp,
        archived_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    purchase_link_session (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        purchase_link_id -> Uuid,
        customer_id -> Uuid,
        checkout_session_id -> Nullable<Uuid>,
        attempt_id -> Text,
        license_instance_id -> Nullable<Text>,
        license_instance_public_key -> Nullable<Text>,
        created_at -> Timestamp,
        cleaned_up_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    purchase_pow_spent (challenge) {
        challenge -> Text,
        expires_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::QuoteStatusEnum;
    use super::sql_types::SubscriptionActivationConditionEnum;

    quote (id) {
        id -> Uuid,
        status -> QuoteStatusEnum,
        created_at -> Timestamptz,
        updated_at -> Nullable<Timestamptz>,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        plan_version_id -> Uuid,
        currency -> Varchar,
        quote_number -> Varchar,
        trial_duration_days -> Nullable<Int4>,
        billing_start_date -> Nullable<Date>,
        billing_end_date -> Nullable<Date>,
        billing_day_anchor -> Nullable<Int4>,
        activation_condition -> SubscriptionActivationConditionEnum,
        valid_until -> Nullable<Timestamptz>,
        expires_at -> Nullable<Timestamptz>,
        accepted_at -> Nullable<Timestamptz>,
        declined_at -> Nullable<Timestamptz>,
        internal_notes -> Nullable<Text>,
        cover_image -> Nullable<Uuid>,
        overview -> Nullable<Text>,
        terms_and_services -> Nullable<Text>,
        net_terms -> Int4,
        attachments -> Array<Nullable<Uuid>>,
        pdf_document_id -> Nullable<Uuid>,
        sharing_key -> Nullable<Varchar>,
        converted_to_invoice_id -> Nullable<Uuid>,
        converted_to_subscription_id -> Nullable<Uuid>,
        converted_at -> Nullable<Timestamptz>,
        recipients -> Jsonb,
        purchase_order -> Nullable<Text>,
        auto_advance_invoices -> Bool,
        charge_automatically -> Bool,
        invoice_memo -> Nullable<Text>,
        invoice_threshold -> Nullable<Numeric>,
        create_subscription_on_acceptance -> Bool,
        payment_methods_config -> Nullable<Jsonb>,
        custom_properties -> Jsonb,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SubscriptionFeeBillingPeriod;

    quote_add_on (id) {
        id -> Uuid,
        name -> Text,
        quote_id -> Uuid,
        add_on_id -> Uuid,
        period -> SubscriptionFeeBillingPeriod,
        legacy_fee -> Nullable<Jsonb>,
        product_id -> Nullable<Uuid>,
        price_id -> Nullable<Uuid>,
        quantity -> Int4,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::AdjustmentKindEnum;

    quote_adjustment (quote_id, kind) {
        quote_id -> Uuid,
        kind -> AdjustmentKindEnum,
        amount_subunit -> Int8,
        includes_all_components -> Bool,
        pending_extra_product_names -> Array<Nullable<Text>>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::AdjustmentKindEnum;

    quote_adjustment_component (quote_id, kind, product_id) {
        quote_id -> Uuid,
        kind -> AdjustmentKindEnum,
        product_id -> Uuid,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SubscriptionFeeBillingPeriod;

    quote_component (id) {
        id -> Uuid,
        name -> Text,
        quote_id -> Uuid,
        price_component_id -> Nullable<Uuid>,
        product_id -> Nullable<Uuid>,
        period -> SubscriptionFeeBillingPeriod,
        legacy_fee -> Nullable<Jsonb>,
        is_override -> Bool,
        price_id -> Nullable<Uuid>,
        example_usage_quantity -> Nullable<Numeric>,
    }
}

diesel::table! {
    quote_coupon (id) {
        id -> Uuid,
        quote_id -> Uuid,
        coupon_id -> Uuid,
    }
}

diesel::table! {
    quote_signature (id) {
        id -> Uuid,
        quote_id -> Uuid,
        signed_by_name -> Varchar,
        signed_by_email -> Varchar,
        signed_by_title -> Nullable<Varchar>,
        signature_data -> Nullable<Text>,
        signature_method -> Varchar,
        signed_at -> Timestamptz,
        ip_address -> Nullable<Varchar>,
        user_agent -> Nullable<Text>,
        verification_token -> Nullable<Varchar>,
        verified_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    role (id) {
        id -> Uuid,
        organization_id -> Uuid,
        name -> Text,
        permissions -> Array<Nullable<Text>>,
        is_system -> Bool,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SandboxPoolStatusEnum;

    sandbox_pool_tenant (tenant_id) {
        tenant_id -> Uuid,
        status -> SandboxPoolStatusEnum,
        created_at -> Timestamp,
        ready_at -> Nullable<Timestamp>,
        claimed_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SandboxSeedStatusEnum;

    sandbox_seed (tenant_id) {
        tenant_id -> Uuid,
        status -> SandboxSeedStatusEnum,
        failure_reason -> Nullable<Text>,
        started_at -> Timestamp,
        finished_at -> Nullable<Timestamp>,
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
    use super::sql_types::ActorTypeEnum;

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
        created_by_customer -> Bool,
        actor_type -> ActorTypeEnum,
        actor_uuid -> Nullable<Uuid>,
        actor_alias -> Nullable<Text>,
        next_attempt_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    segment (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        name -> Text,
        definition -> Jsonb,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    sent_email (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        sent_at -> Timestamptz,
        subject -> Text,
        from_addr -> Text,
        reply_to -> Nullable<Text>,
        recipients -> Array<Nullable<Text>>,
        body_html -> Text,
        attachments -> Nullable<Jsonb>,
        kind -> Nullable<Text>,
        locale -> Nullable<Text>,
        invoicing_entity_id -> Nullable<Uuid>,
        context -> Nullable<Jsonb>,
        dedupe_key -> Nullable<Text>,
        resent_from -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::SlotTransactionStatus;

    slot_transaction (id) {
        id -> Uuid,
        subscription_id -> Uuid,
        delta -> Int4,
        prev_active_slots -> Int4,
        effective_at -> Timestamp,
        transaction_at -> Timestamp,
        #[max_length = 255]
        unit -> Varchar,
        status -> SlotTransactionStatus,
        invoice_id -> Nullable<Uuid>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::StoredObjectKindEnum;

    stored_object (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        kind -> StoredObjectKindEnum,
        key -> Text,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::BillingPeriodEnum;
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
        net_terms -> Int4,
        invoice_memo -> Nullable<Text>,
        invoice_threshold -> Nullable<Numeric>,
        activated_at -> Nullable<Timestamp>,
        #[max_length = 3]
        currency -> Varchar,
        mrr_cents -> Int8,
        period -> BillingPeriodEnum,
        pending_checkout -> Bool,
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
        purchase_order -> Nullable<Text>,
        quote_id -> Nullable<Uuid>,
        backdate_invoices -> Bool,
        processing_started_at -> Nullable<Timestamp>,
        payment_methods_config -> Nullable<Jsonb>,
        imported_at -> Nullable<Timestamp>,
        custom_properties -> Jsonb,
        terminated_at -> Nullable<Timestamp>,
        tax_inclusive -> Bool,
        tax_inclusive_rate -> Nullable<Numeric>,
        usage_filter_property -> Nullable<Text>,
        usage_filter_value -> Nullable<Text>,
        usage_filter_since -> Nullable<Timestamp>,
        lifecycle_suspended -> Bool,
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
        legacy_fee -> Nullable<Jsonb>,
        created_at -> Timestamp,
        product_id -> Nullable<Uuid>,
        price_id -> Nullable<Uuid>,
        quantity -> Int4,
        effective_from -> Date,
        effective_to -> Nullable<Date>,
        lineage_id -> Nullable<Uuid>,
        added_by_amendment -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::AdjustmentKindEnum;

    subscription_adjustment (subscription_id, kind) {
        subscription_id -> Uuid,
        kind -> AdjustmentKindEnum,
        amount_subunit -> Int8,
        includes_all_components -> Bool,
        is_override -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::AdjustmentKindEnum;

    subscription_adjustment_component (subscription_id, kind, product_id) {
        subscription_id -> Uuid,
        kind -> AdjustmentKindEnum,
        product_id -> Uuid,
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
        legacy_fee -> Nullable<Jsonb>,
        price_id -> Nullable<Uuid>,
        effective_from -> Date,
        effective_to -> Nullable<Date>,
        lineage_id -> Nullable<Uuid>,
        added_by_amendment -> Bool,
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
        outgoing_checked_at -> Nullable<Timestamp>,
        outgoing_next_check_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    subscription_usage_filter (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        subscription_id -> Uuid,
        property -> Nullable<Text>,
        value -> Nullable<Text>,
        effective_from -> Timestamp,
        effective_until -> Nullable<Timestamp>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    tax_category (id) {
        id -> Uuid,
        tenant_id -> Nullable<Uuid>,
        parent_id -> Nullable<Uuid>,
        key -> Text,
        name -> Text,
        is_builtin -> Bool,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::TenantEnvironmentEnum;
    use super::sql_types::TenantStatusEnum;
    use super::sql_types::EmailDeliveryModeEnum;

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
        disable_emails -> Bool,
        last_active_at -> Nullable<Timestamp>,
        status -> TenantStatusEnum,
        alert_evaluation_interval_secs -> Nullable<Int4>,
        email_delivery_mode -> EmailDeliveryModeEnum,
        email_test_recipients -> Array<Nullable<Text>>,
        keep_last_events -> Bool,
    }
}

diesel::table! {
    usage_alert_state (monitor_id, window_key, level) {
        monitor_id -> Uuid,
        window_key -> Text,
        level -> Int4,
        tenant_id -> Uuid,
        fired_at -> Timestamptz,
        generation -> Int4,
        rearmed -> Bool,
        window_start -> Nullable<Timestamptz>,
        window_end -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    usage_fold (customer_id, billable_metric_id, window_start, grouping, group_key) {
        customer_id -> Uuid,
        billable_metric_id -> Uuid,
        group_key -> Text,
        tenant_id -> Uuid,
        fold_at -> Timestamptz,
        quantity -> Numeric,
        updated_at -> Timestamptz,
        metric_version -> Int8,
        rebuilt_at -> Timestamptz,
        grouping -> Text,
        window_start -> Timestamptz,
    }
}

diesel::table! {
    usage_monitor_state (monitor_id, window_start, group_key) {
        monitor_id -> Uuid,
        window_start -> Timestamptz,
        group_key -> Text,
        tenant_id -> Uuid,
        customer_id -> Uuid,
        monitor_kind -> Text,
        window_end -> Timestamptz,
        used -> Numeric,
        limit_value -> Nullable<Numeric>,
        usage_as_of -> Timestamptz,
        computed_at -> Timestamptz,
        metric_ids -> Nullable<Array<Nullable<Uuid>>>,
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
        totp_secret -> Nullable<Text>,
        totp_enabled -> Bool,
        totp_recovery_codes -> Array<Nullable<Text>>,
        totp_enrolled_at -> Nullable<Timestamp>,
        preferences -> Jsonb,
    }
}

diesel::table! {
    webhook_in_event (id) {
        id -> Uuid,
        received_at -> Timestamptz,
        action -> Nullable<Text>,
        key -> Text,
        attempts -> Int4,
        error -> Nullable<Text>,
        provider_config_id -> Uuid,
        event_id -> Nullable<Text>,
        processed_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    webhook_out_attempt (id) {
        id -> Uuid,
        delivery_id -> Uuid,
        attempt_number -> Int4,
        attempted_at -> Timestamp,
        duration_ms -> Int4,
        response_status -> Nullable<Int2>,
        response_body -> Nullable<Text>,
        error -> Nullable<Text>,
        succeeded -> Bool,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::WebhookDeliveryStatusEnum;

    webhook_out_delivery (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        message_id -> Uuid,
        endpoint_id -> Uuid,
        status -> WebhookDeliveryStatusEnum,
        attempt_count -> Int4,
        next_attempt_at -> Nullable<Timestamp>,
        lease_expires_at -> Nullable<Timestamp>,
        last_response_status -> Nullable<Int2>,
        last_error -> Nullable<Text>,
        manual -> Bool,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        completed_at -> Nullable<Timestamp>,
        claimed_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::WebhookEndpointDisabledReasonEnum;

    webhook_out_endpoint (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        url -> Text,
        description -> Nullable<Text>,
        event_types -> Array<Nullable<Text>>,
        secret -> Text,
        previous_secret -> Nullable<Text>,
        previous_secret_expires_at -> Nullable<Timestamp>,
        disabled -> Bool,
        disabled_reason -> Nullable<WebhookEndpointDisabledReasonEnum>,
        disabled_at -> Nullable<Timestamp>,
        consecutive_failures -> Int4,
        last_success_at -> Nullable<Timestamp>,
        last_failure_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
        updated_at -> Nullable<Timestamp>,
        archived_at -> Nullable<Timestamp>,
        max_in_flight -> Int4,
        headers -> Jsonb,
        sensitive_header_values -> Nullable<Text>,
        failing_since -> Nullable<Timestamp>,
        unreachable_streak -> Int4,
        paused_until -> Nullable<Timestamp>,
        rate_limit_per_sec -> Nullable<Int4>,
        legacy_svix_headers -> Bool,
    }
}

diesel::table! {
    webhook_out_message (id) {
        id -> Uuid,
        tenant_id -> Uuid,
        event_type -> Text,
        payload -> Jsonb,
        created_at -> Timestamp,
    }
}

diesel::joinable!(add_on -> price (price_id));
diesel::joinable!(add_on -> product (product_id));
diesel::joinable!(add_on -> tenant (tenant_id));
diesel::joinable!(alert_definition -> tenant (tenant_id));
diesel::joinable!(alert_definition -> user (owner_user_id));
diesel::joinable!(alert_event -> alert_definition (alert_definition_id));
diesel::joinable!(alert_event -> customer (customer_id));
diesel::joinable!(alert_event -> tenant (tenant_id));
diesel::joinable!(alert_segment_member -> alert_definition (alert_definition_id));
diesel::joinable!(alert_segment_member -> customer (customer_id));
diesel::joinable!(alert_segment_member -> tenant (tenant_id));
diesel::joinable!(analytics_dashboard_share -> analytics_saved_view (saved_view_id));
diesel::joinable!(analytics_dashboard_share -> tenant (tenant_id));
diesel::joinable!(analytics_projection_quarantine -> tenant (tenant_id));
diesel::joinable!(analytics_saved_view -> tenant (tenant_id));
diesel::joinable!(api_idempotency_key -> tenant (tenant_id));
diesel::joinable!(api_token -> tenant (tenant_id));
diesel::joinable!(applied_coupon -> coupon (coupon_id));
diesel::joinable!(applied_coupon -> customer (customer_id));
diesel::joinable!(applied_coupon -> subscription (subscription_id));
diesel::joinable!(bank_account -> tenant (tenant_id));
diesel::joinable!(batch_job -> tenant (tenant_id));
diesel::joinable!(batch_job_chunk -> batch_job (job_id));
diesel::joinable!(batch_job_chunk -> tenant (tenant_id));
diesel::joinable!(batch_job_entity -> batch_job (batch_job_id));
diesel::joinable!(batch_job_entity -> tenant (tenant_id));
diesel::joinable!(batch_job_item_failure -> batch_job (job_id));
diesel::joinable!(batch_job_item_failure -> batch_job_chunk (chunk_id));
diesel::joinable!(batch_job_item_failure -> tenant (tenant_id));
diesel::joinable!(bi_mrr_movement_log -> credit_note (credit_note_id));
diesel::joinable!(bi_mrr_movement_log -> customer (customer_id));
diesel::joinable!(bi_mrr_movement_log -> invoice (invoice_id));
diesel::joinable!(bi_mrr_movement_log -> plan_version (plan_version_id));
diesel::joinable!(bi_mrr_movement_log -> subscription (subscription_id));
diesel::joinable!(bi_mrr_movement_log -> tenant (tenant_id));
diesel::joinable!(billable_metric -> product (product_id));
diesel::joinable!(billable_metric -> product_family (product_family_id));
diesel::joinable!(billable_metric -> tenant (tenant_id));
diesel::joinable!(checkout_session -> customer (customer_id));
diesel::joinable!(checkout_session -> plan_version (plan_version_id));
diesel::joinable!(checkout_session -> subscription (subscription_id));
diesel::joinable!(checkout_session -> tenant (tenant_id));
diesel::joinable!(connect_access_token -> connect_oauth_app (oauth_app_id));
diesel::joinable!(connect_access_token -> organization (connected_organization_id));
diesel::joinable!(connect_authorization_code -> connect_oauth_app (oauth_app_id));
diesel::joinable!(connect_authorization_code -> organization (connected_organization_id));
diesel::joinable!(connect_connected_account -> tenant (connected_tenant_id));
diesel::joinable!(connect_oauth_app -> organization (organization_id));
diesel::joinable!(connect_onboarding_link -> connect_connected_account (connected_account_id));
diesel::joinable!(connect_platform_settings -> invoicing_entity (branding_invoicing_entity_id));
diesel::joinable!(connect_platform_settings -> organization (organization_id));
diesel::joinable!(connect_refresh_token -> connect_access_token (access_token_id));
diesel::joinable!(connector -> tenant (tenant_id));
diesel::joinable!(coupon -> tenant (tenant_id));
diesel::joinable!(coupon_plan -> coupon (coupon_id));
diesel::joinable!(coupon_plan -> plan (plan_id));
diesel::joinable!(credit_balance_projection -> tenant (tenant_id));
diesel::joinable!(credit_customer_inputs -> customer (customer_id));
diesel::joinable!(credit_customer_inputs -> tenant (tenant_id));
diesel::joinable!(credit_note -> customer (customer_id));
diesel::joinable!(credit_note -> invoice (invoice_id));
diesel::joinable!(credit_note -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(credit_note -> plan_version (plan_version_id));
diesel::joinable!(credit_note -> subscription (subscription_id));
diesel::joinable!(credit_note -> tenant (tenant_id));
diesel::joinable!(credit_recompute_mark -> customer (customer_id));
diesel::joinable!(credit_recompute_mark -> tenant (tenant_id));
diesel::joinable!(credit_tenant_inputs -> tenant (tenant_id));
diesel::joinable!(credit_unit -> product (product_id));
diesel::joinable!(credit_unit -> tenant (tenant_id));
diesel::joinable!(credit_unit_price -> tenant (tenant_id));
diesel::joinable!(credit_usage_settlement -> customer (customer_id));
diesel::joinable!(credit_usage_settlement -> invoice (invoice_id));
diesel::joinable!(credit_usage_settlement -> subscription (subscription_id));
diesel::joinable!(credit_usage_settlement -> tenant (tenant_id));
diesel::joinable!(custom_property_definition -> tenant (tenant_id));
diesel::joinable!(custom_tax -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(custom_tax -> tax_category (tax_category_id));
diesel::joinable!(custom_tax -> tenant (tenant_id));
diesel::joinable!(customer -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(customer -> payment_reminder_sequence (payment_reminder_sequence_id));
diesel::joinable!(customer -> tenant (tenant_id));
diesel::joinable!(customer -> user (owner_user_id));
diesel::joinable!(customer_balance_pending_tx -> customer (customer_id));
diesel::joinable!(customer_balance_pending_tx -> customer_balance_tx (tx_id));
diesel::joinable!(customer_balance_pending_tx -> invoice (invoice_id));
diesel::joinable!(customer_balance_pending_tx -> tenant (tenant_id));
diesel::joinable!(customer_balance_pending_tx -> user (created_by));
diesel::joinable!(customer_balance_tx -> customer (customer_id));
diesel::joinable!(customer_balance_tx -> invoice (invoice_id));
diesel::joinable!(customer_balance_tx -> tenant (tenant_id));
diesel::joinable!(customer_balance_tx -> user (created_by));
diesel::joinable!(customer_budget -> customer (customer_id));
diesel::joinable!(customer_budget -> subscription (subscription_id));
diesel::joinable!(customer_budget -> tenant (tenant_id));
diesel::joinable!(customer_connection -> connector (connector_id));
diesel::joinable!(customer_connection -> customer (customer_id));
diesel::joinable!(customer_credit_account -> customer (customer_id));
diesel::joinable!(customer_credit_account -> tenant (tenant_id));
diesel::joinable!(customer_credit_auto_top_up -> credit_unit_price (agreed_price_id));
diesel::joinable!(customer_credit_auto_top_up -> customer_payment_method (payment_method_id));
diesel::joinable!(customer_credit_auto_top_up -> tenant (tenant_id));
diesel::joinable!(customer_credit_entry -> customer_credit_grant (grant_id));
diesel::joinable!(customer_credit_entry -> tenant (tenant_id));
diesel::joinable!(customer_credit_grant -> customer (customer_id));
diesel::joinable!(customer_credit_grant -> subscription (subscription_id));
diesel::joinable!(customer_credit_grant -> tenant (tenant_id));
diesel::joinable!(customer_credit_grant_product -> customer_credit_grant (grant_id));
diesel::joinable!(customer_credit_grant_product -> product (product_id));
diesel::joinable!(customer_credit_grant_product -> tenant (tenant_id));
diesel::joinable!(customer_credit_line_allocation -> customer_credit_grant (grant_id));
diesel::joinable!(customer_credit_line_allocation -> tenant (tenant_id));
diesel::joinable!(customer_credit_operation -> customer (customer_id));
diesel::joinable!(customer_credit_operation -> invoice (invoice_id));
diesel::joinable!(customer_credit_operation -> payment_transaction (payment_transaction_id));
diesel::joinable!(customer_credit_operation -> tenant (tenant_id));
diesel::joinable!(customer_credit_purchase -> customer (customer_id));
diesel::joinable!(customer_credit_purchase -> invoice (invoice_id));
diesel::joinable!(customer_credit_purchase -> tenant (tenant_id));
diesel::joinable!(customer_credit_unrecovered -> tenant (tenant_id));
diesel::joinable!(customer_milestone -> customer (customer_id));
diesel::joinable!(customer_milestone -> tenant (tenant_id));
diesel::joinable!(customer_payment_method -> customer_connection (connection_id));
diesel::joinable!(customer_payment_method -> tenant (tenant_id));
diesel::joinable!(dead_letter_message -> tenant (tenant_id));
diesel::joinable!(email_kind_audience -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(email_kind_audience -> tenant (tenant_id));
diesel::joinable!(email_send_claim -> sent_email (sent_email_id));
diesel::joinable!(email_send_claim -> tenant (tenant_id));
diesel::joinable!(entitlement -> feature (feature_id));
diesel::joinable!(entitlement -> tenant (tenant_id));
diesel::joinable!(entity_activity -> tenant (tenant_id));
diesel::joinable!(entity_list_view -> tenant (tenant_id));
diesel::joinable!(entity_list_view -> user (created_by));
diesel::joinable!(entity_list_view_default -> entity_list_view (view_id));
diesel::joinable!(entity_list_view_default -> tenant (tenant_id));
diesel::joinable!(entity_list_view_default -> user (user_id));
diesel::joinable!(feature -> billable_metric (metric_id));
diesel::joinable!(feature -> product (product_id));
diesel::joinable!(feature -> tenant (tenant_id));
diesel::joinable!(invoice -> customer (customer_id));
diesel::joinable!(invoice -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(invoice -> plan_version (plan_version_id));
diesel::joinable!(invoice -> tenant (tenant_id));
diesel::joinable!(invoice_annex_template -> tenant (tenant_id));
diesel::joinable!(invoice_payment_reminder -> invoice (invoice_id));
diesel::joinable!(invoice_payment_reminder -> payment_reminder_sequence (sequence_id));
diesel::joinable!(invoice_payment_reminder -> tenant (tenant_id));
diesel::joinable!(invoicing_entity -> bank_account (bank_account_id));
diesel::joinable!(invoicing_entity -> tax_category (default_tax_category_id));
diesel::joinable!(invoicing_entity -> tenant (tenant_id));
diesel::joinable!(invoicing_entity_email_settings -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(invoicing_entity_email_settings -> tenant (tenant_id));
diesel::joinable!(issued_license -> customer (customer_id));
diesel::joinable!(issued_license -> subscription (subscription_id));
diesel::joinable!(issued_license -> tenant (tenant_id));
diesel::joinable!(license_signing_key -> tenant (tenant_id));
diesel::joinable!(licensing_endpoint -> tenant (tenant_id));
diesel::joinable!(metric_target -> tenant (tenant_id));
diesel::joinable!(metric_target -> user (owner_user_id));
diesel::joinable!(metric_target_change -> tenant (tenant_id));
diesel::joinable!(metric_target_change -> user (actor_user_id));
diesel::joinable!(metric_target_snapshot -> metric_target (metric_target_id));
diesel::joinable!(metric_target_snapshot -> tenant (tenant_id));
diesel::joinable!(notification_channel -> tenant (tenant_id));
diesel::joinable!(org_security_audit_log -> organization (organization_id));
diesel::joinable!(organization_domain -> organization (organization_id));
diesel::joinable!(organization_invite -> organization (organization_id));
diesel::joinable!(organization_invite -> role (role_id));
diesel::joinable!(organization_invite -> user (invited_by));
diesel::joinable!(organization_member -> organization (organization_id));
diesel::joinable!(organization_member -> role (role_id));
diesel::joinable!(organization_member -> user (user_id));
diesel::joinable!(organization_milestone -> organization (organization_id));
diesel::joinable!(organization_milestone -> tenant (tenant_id));
diesel::joinable!(organization_sso_connection -> organization (organization_id));
diesel::joinable!(outbox_event -> tenant (tenant_id));
diesel::joinable!(payment_reminder_sequence -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(payment_reminder_sequence -> tenant (tenant_id));
diesel::joinable!(payment_reminder_step -> payment_reminder_sequence (sequence_id));
diesel::joinable!(payment_reminder_step -> tenant (tenant_id));
diesel::joinable!(payment_reversal_movement -> customer (customer_id));
diesel::joinable!(payment_reversal_movement -> invoice (invoice_id));
diesel::joinable!(payment_reversal_movement -> payment_transaction (payment_transaction_id));
diesel::joinable!(payment_reversal_movement -> tenant (tenant_id));
diesel::joinable!(payment_transaction -> checkout_session (checkout_session_id));
diesel::joinable!(payment_transaction -> credit_note (credit_note_id));
diesel::joinable!(payment_transaction -> customer_connection (pending_connection_id));
diesel::joinable!(payment_transaction -> customer_payment_method (payment_method_id));
diesel::joinable!(payment_transaction -> invoice (invoice_id));
diesel::joinable!(payment_transaction -> plan_version (pending_plan_version_id));
diesel::joinable!(payment_transaction -> tenant (tenant_id));
diesel::joinable!(plan -> product_family (product_family_id));
diesel::joinable!(plan -> tenant (tenant_id));
diesel::joinable!(plan_component_price -> price (price_id));
diesel::joinable!(plan_component_price -> price_component (plan_component_id));
diesel::joinable!(plan_version -> tenant (tenant_id));
diesel::joinable!(plan_version_add_on -> add_on (add_on_id));
diesel::joinable!(plan_version_add_on -> plan_version (plan_version_id));
diesel::joinable!(plan_version_add_on -> price (price_id));
diesel::joinable!(plan_version_add_on -> tenant (tenant_id));
diesel::joinable!(plan_version_adjustment -> plan_version (plan_version_id));
diesel::joinable!(plan_version_adjustment_component -> product (product_id));
diesel::joinable!(plan_version_annex_template -> invoice_annex_template (annex_template_id));
diesel::joinable!(plan_version_annex_template -> plan_version (plan_version_id));
diesel::joinable!(plan_version_annex_template -> tenant (tenant_id));
diesel::joinable!(price -> product (product_id));
diesel::joinable!(price -> tenant (tenant_id));
diesel::joinable!(price_component -> billable_metric (billable_metric_id));
diesel::joinable!(price_component -> plan_version (plan_version_id));
diesel::joinable!(price_component -> product (product_id));
diesel::joinable!(product -> product_family (product_family_id));
diesel::joinable!(product -> tax_category (tax_category_id));
diesel::joinable!(product -> tenant (tenant_id));
diesel::joinable!(product_accounting -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(product_accounting -> product (product_id));
diesel::joinable!(product_custom_tax -> custom_tax (custom_tax_id));
diesel::joinable!(product_custom_tax -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(product_custom_tax -> product (product_id));
diesel::joinable!(product_family -> tenant (tenant_id));
diesel::joinable!(promo_code_attempt -> customer (customer_id));
diesel::joinable!(promo_code_attempt -> tenant (tenant_id));
diesel::joinable!(purchase_link -> plan (plan_id));
diesel::joinable!(purchase_link -> tenant (tenant_id));
diesel::joinable!(purchase_link_session -> checkout_session (checkout_session_id));
diesel::joinable!(purchase_link_session -> customer (customer_id));
diesel::joinable!(purchase_link_session -> purchase_link (purchase_link_id));
diesel::joinable!(purchase_link_session -> tenant (tenant_id));
diesel::joinable!(quote -> customer (customer_id));
diesel::joinable!(quote -> invoice (converted_to_invoice_id));
diesel::joinable!(quote -> plan_version (plan_version_id));
diesel::joinable!(quote -> tenant (tenant_id));
diesel::joinable!(quote_add_on -> add_on (add_on_id));
diesel::joinable!(quote_add_on -> price (price_id));
diesel::joinable!(quote_add_on -> product (product_id));
diesel::joinable!(quote_add_on -> quote (quote_id));
diesel::joinable!(quote_adjustment -> quote (quote_id));
diesel::joinable!(quote_component -> price (price_id));
diesel::joinable!(quote_component -> price_component (price_component_id));
diesel::joinable!(quote_component -> product (product_id));
diesel::joinable!(quote_component -> quote (quote_id));
diesel::joinable!(quote_coupon -> coupon (coupon_id));
diesel::joinable!(quote_coupon -> quote (quote_id));
diesel::joinable!(quote_signature -> quote (quote_id));
diesel::joinable!(role -> organization (organization_id));
diesel::joinable!(sandbox_pool_tenant -> tenant (tenant_id));
diesel::joinable!(sandbox_seed -> tenant (tenant_id));
diesel::joinable!(schedule -> plan_version (plan_version_id));
diesel::joinable!(scheduled_event -> subscription (subscription_id));
diesel::joinable!(segment -> tenant (tenant_id));
diesel::joinable!(sent_email -> entity_activity (id));
diesel::joinable!(sent_email -> invoicing_entity (invoicing_entity_id));
diesel::joinable!(sent_email -> tenant (tenant_id));
diesel::joinable!(slot_transaction -> invoice (invoice_id));
diesel::joinable!(slot_transaction -> subscription (subscription_id));
diesel::joinable!(stored_object -> tenant (tenant_id));
diesel::joinable!(subscription -> customer (customer_id));
diesel::joinable!(subscription -> plan_version (plan_version_id));
diesel::joinable!(subscription -> tenant (tenant_id));
diesel::joinable!(subscription_add_on -> add_on (add_on_id));
diesel::joinable!(subscription_add_on -> price (price_id));
diesel::joinable!(subscription_add_on -> product (product_id));
diesel::joinable!(subscription_add_on -> subscription (subscription_id));
diesel::joinable!(subscription_adjustment -> subscription (subscription_id));
diesel::joinable!(subscription_component -> price (price_id));
diesel::joinable!(subscription_component -> price_component (price_component_id));
diesel::joinable!(subscription_component -> product (product_id));
diesel::joinable!(subscription_component -> subscription (subscription_id));
diesel::joinable!(subscription_event -> bi_mrr_movement_log (bi_mrr_movement_log_id));
diesel::joinable!(subscription_event -> subscription (subscription_id));
diesel::joinable!(subscription_usage_filter -> subscription (subscription_id));
diesel::joinable!(subscription_usage_filter -> tenant (tenant_id));
diesel::joinable!(tax_category -> tenant (tenant_id));
diesel::joinable!(tenant -> organization (organization_id));
diesel::joinable!(usage_alert_state -> tenant (tenant_id));
diesel::joinable!(usage_fold -> billable_metric (billable_metric_id));
diesel::joinable!(usage_fold -> customer (customer_id));
diesel::joinable!(usage_fold -> tenant (tenant_id));
diesel::joinable!(usage_monitor_state -> tenant (tenant_id));
diesel::joinable!(webhook_in_event -> connector (provider_config_id));
diesel::joinable!(webhook_out_attempt -> webhook_out_delivery (delivery_id));
diesel::joinable!(webhook_out_delivery -> tenant (tenant_id));
diesel::joinable!(webhook_out_delivery -> webhook_out_endpoint (endpoint_id));
diesel::joinable!(webhook_out_delivery -> webhook_out_message (message_id));
diesel::joinable!(webhook_out_endpoint -> tenant (tenant_id));
diesel::joinable!(webhook_out_message -> tenant (tenant_id));
