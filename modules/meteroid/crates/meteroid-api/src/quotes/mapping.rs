pub mod quotes {
    use meteroid_app::conversions::{AsProtoOpt, ProtoConv};

    use crate::customers::mapping::customer::ServerCustomerWrapper;
    use crate::subscriptions::mapping::price_components::{
        subscription_fee_billing_period_to_grpc, subscription_fee_to_grpc,
    };
    use meteroid_grpc::meteroid::api::quotes::v1;
    use meteroid_grpc::meteroid::api::quotes::v1::{
        DetailedQuote, Quote, QuoteComponent, QuoteSignature, QuoteStatus, RecipientDetails,
    };
    use meteroid_grpc::meteroid::api::subscriptions::v1 as sub_proto;
    use meteroid_grpc::meteroid::api::subscriptions::v1::ActivationCondition;
    use meteroid_store::domain;
    use meteroid_store::domain::subscriptions::PaymentMethodsConfig;
    use meteroid_store::services::invoice_lines::fees::compute_usage_price;

    fn status_domain_to_server(value: domain::enums::QuoteStatusEnum) -> QuoteStatus {
        match value {
            domain::enums::QuoteStatusEnum::Draft => QuoteStatus::Draft,
            domain::enums::QuoteStatusEnum::Pending => QuoteStatus::Pending,
            domain::enums::QuoteStatusEnum::Accepted => QuoteStatus::Accepted,
            domain::enums::QuoteStatusEnum::Declined => QuoteStatus::Declined,
            domain::enums::QuoteStatusEnum::Expired => QuoteStatus::Expired,
            domain::enums::QuoteStatusEnum::Cancelled => QuoteStatus::Cancelled,
        }
    }

    pub fn status_server_to_domain(status: Option<i32>) -> Option<domain::enums::QuoteStatusEnum> {
        status.and_then(|status_int| {
            QuoteStatus::try_from(status_int)
                .ok()
                .map(|status| match status {
                    QuoteStatus::Draft => domain::enums::QuoteStatusEnum::Draft,
                    QuoteStatus::Pending => domain::enums::QuoteStatusEnum::Pending,
                    QuoteStatus::Accepted => domain::enums::QuoteStatusEnum::Accepted,
                    QuoteStatus::Declined => domain::enums::QuoteStatusEnum::Declined,
                    QuoteStatus::Expired => domain::enums::QuoteStatusEnum::Expired,
                    QuoteStatus::Cancelled => domain::enums::QuoteStatusEnum::Cancelled,
                })
        })
    }

    fn recipient_details_to_proto(
        recipient: &domain::quotes::RecipientDetails,
    ) -> RecipientDetails {
        RecipientDetails {
            name: recipient.name.clone(),
            email: recipient.email.clone(),
        }
    }

    fn payment_methods_config_to_proto(
        config: Option<PaymentMethodsConfig>,
    ) -> Option<sub_proto::PaymentMethodsConfig> {
        config.map(|c| match c {
            PaymentMethodsConfig::Online { config } => sub_proto::PaymentMethodsConfig {
                config: Some(sub_proto::payment_methods_config::Config::Online(
                    sub_proto::OnlinePayment {
                        config: config.map(|cfg| sub_proto::OnlineMethodsConfig {
                            card: cfg
                                .card
                                .map(|m| sub_proto::OnlineMethodConfig { enabled: m.enabled }),
                            direct_debit: cfg
                                .direct_debit
                                .map(|m| sub_proto::OnlineMethodConfig { enabled: m.enabled }),
                        }),
                    },
                )),
            },
            PaymentMethodsConfig::BankTransfer { account_id: _ } => {
                sub_proto::PaymentMethodsConfig {
                    config: Some(sub_proto::payment_methods_config::Config::BankTransfer(
                        sub_proto::BankTransfer {},
                    )),
                }
            }
            PaymentMethodsConfig::External => sub_proto::PaymentMethodsConfig {
                config: Some(sub_proto::payment_methods_config::Config::External(
                    sub_proto::External {},
                )),
            },
        })
    }

    pub(crate) fn quote_component_to_proto(
        component: &domain::quotes::QuotePriceComponent,
        currency: &str,
    ) -> QuoteComponent {
        let example_usage_amount = match (&component.fee, component.example_usage_quantity) {
            (domain::SubscriptionFee::Usage { model, .. }, Some(qty)) => {
                compute_usage_price(model, qty, currency)
                    .ok()
                    .flatten()
                    .map(|a| a.normalize().to_string())
            }
            _ => None,
        };

        QuoteComponent {
            id: component.id.as_proto(),
            name: component.name.clone(),
            price_component_id: component.price_component_id.map(|id| id.as_proto()),
            product_id: component.product_id.map(|id| id.as_proto()),
            period: subscription_fee_billing_period_to_grpc(component.period) as i32,
            fee: Some(subscription_fee_to_grpc(
                &component.fee,
                component.period.as_billing_period_opt().unwrap_or_default(),
            )),
            is_override: component.is_override,
            example_usage_quantity: component.example_usage_quantity.map(|q| q.to_string()),
            example_usage_amount,
            description: component.description.clone(),
            sort_order: component.sort_order,
            discount_percent: component
                .discount_percent
                .map(|d| d.normalize().to_string()),
            list_fee: component.list_fee.as_ref().map(|f| {
                subscription_fee_to_grpc(
                    f,
                    component.period.as_billing_period_opt().unwrap_or_default(),
                )
            }),
        }
    }

    fn quote_signature_to_proto(signature: &domain::quotes::QuoteSignature) -> QuoteSignature {
        QuoteSignature {
            id: signature.id.as_proto(),
            quote_id: signature.quote_id.as_proto(),
            signed_by_name: signature.signed_by_name.clone(),
            signed_by_email: signature.signed_by_email.clone(),
            signed_by_title: signature.signed_by_title.clone(),
            signature_data: signature.signature_data.clone(),
            signature_method: signature.signature_method.clone(),
            signed_at: signature.signed_at.as_proto(),
            ip_address: signature.ip_address.clone(),
            user_agent: signature.user_agent.clone(),
            verification_token: signature.verification_token.clone(),
            verified_at: signature.verified_at.as_proto(),
            signer_role: signer_role_to_proto(signature.signer_role) as i32,
            revision: signature.revision,
            document_hash: signature.document_hash.clone(),
            consent_text: signature.consent_text.clone(),
            code_verified_at: signature.otp_verified_at.as_proto(),
        }
    }

    pub fn quote_to_proto(
        quote: &domain::quotes::Quote,
        customer_name: Option<String>,
        for_portal: bool,
    ) -> Quote {
        let proto = Quote {
            id: quote.id.as_proto(),
            quote_number: quote.quote_number.clone(),
            status: status_domain_to_server(quote.status) as i32,
            created_at: quote.created_at.as_proto(),
            updated_at: quote.updated_at.as_proto(),
            customer_id: quote.customer_id.as_proto(),
            customer_name,
            plan_version_id: quote.plan_version_id.as_proto(),
            currency: quote.currency.clone(),
            net_terms: quote.net_terms,
            // Subscription-like fields
            trial_duration: quote.trial_duration_days.map(|d| d as u32),
            start_date: quote.billing_start_date.as_proto(),
            billing_start_date: quote.billing_start_date.as_proto(),
            end_date: quote.billing_end_date.as_proto(),
            billing_day_anchor: quote.billing_day_anchor.map(|d| d as u32),
            activation_condition: activation_condition_to_proto(quote.activation_condition.clone())
                as i32,
            // Quote-specific fields
            valid_until: quote.valid_until.as_proto(),
            expires_at: quote.expires_at.as_proto(),
            accepted_at: quote.accepted_at.as_proto(),
            declined_at: quote.declined_at.as_proto(),
            internal_notes: quote.internal_notes.clone(),
            cover_image: quote.cover_image.map(|id| id.as_proto()),
            overview: quote.overview.clone(),
            terms_and_services: quote.terms_and_services.clone(),
            recipients: quote
                .recipients
                .iter()
                .map(recipient_details_to_proto)
                .collect(),
            purchase_order: quote.purchase_order.clone(),
            // Payment configuration fields
            auto_advance_invoices: quote.auto_advance_invoices,
            charge_automatically: quote.charge_automatically,
            invoice_memo: quote.invoice_memo.clone(),
            invoice_threshold: quote.invoice_threshold.map(|d| d.to_string()),
            create_subscription_on_acceptance: quote.create_subscription_on_acceptance,
            payment_methods_config: payment_methods_config_to_proto(
                quote.payment_methods_config.clone(),
            ),
            // Conversion tracking
            converted_to_subscription_id: quote
                .converted_to_subscription_id
                .map(|id| id.as_proto()),
            converted_at: quote.converted_at.as_proto(),
            custom_properties: (!quote.custom_properties.is_null())
                .then(|| quote.custom_properties.to_string()),
            revision: quote.revision,
            document_hash: quote.document_hash.clone(),
            published_at: quote.published_at.as_proto(),
            first_viewed_at: quote.first_viewed_at.as_proto(),
            owner_id: quote.owner_id.map(|id| id.as_proto()),
            template_id: quote.template_id.map(|id| id.as_proto()),
            decline_reason: quote.decline_reason.clone(),
            display_options: Some(display_options_to_proto(&quote.display_options)),
            sections: quote.sections.iter().map(section_to_proto).collect(),
            signature_level: signature_level_to_proto(quote.signature_level) as i32,
            consent_text: quote.consent_text.clone(),
            countersigners: quote
                .countersigners
                .iter()
                .map(countersigner_to_proto)
                .collect(),
            approval_status: approval_status_to_proto(quote.approval_status) as i32,
            hubspot_deal_id: quote.hubspot_deal_id.clone(),
            max_discount_percent: quote
                .max_discount_percent
                .map(|d| d.normalize().to_string()),
            changes_requested_at: quote.changes_requested_at.as_proto(),
            customer_actions: Some(v1::QuoteCustomerActions {
                questions: quote.allow_questions,
                change_requests: quote.allow_change_requests,
                decline: quote.allow_decline,
            }),
            last_sent_at: quote.last_sent_at.as_proto(),
            attachments: quote.attachments.iter().map(attachment_to_proto).collect(),
        };
        if !for_portal {
            return proto;
        }
        // The seller's own data; the portal details carry what a signer needs of countersigners.
        Quote {
            internal_notes: None,
            owner_id: None,
            template_id: None,
            countersigners: vec![],
            approval_status: 0,
            hubspot_deal_id: None,
            max_discount_percent: None,
            ..proto
        }
    }

    pub fn attachment_to_proto(
        attachment: &domain::quotes::QuoteAttachment,
    ) -> v1::QuoteAttachment {
        v1::QuoteAttachment {
            id: attachment.id.as_proto(),
            filename: attachment.filename.clone(),
            content_type: attachment.content_type.clone(),
            size_bytes: attachment.size_bytes,
            sha256: attachment.sha256.clone(),
            download_url: None,
        }
    }

    pub fn signer_role_to_proto(role: domain::enums::QuoteSignerRoleEnum) -> v1::QuoteSignerRole {
        match role {
            domain::enums::QuoteSignerRoleEnum::Customer => v1::QuoteSignerRole::SignerCustomer,
            domain::enums::QuoteSignerRoleEnum::Countersigner => {
                v1::QuoteSignerRole::SignerCountersigner
            }
        }
    }

    pub fn signature_level_to_proto(
        level: domain::enums::QuoteSignatureLevelEnum,
    ) -> v1::QuoteSignatureLevel {
        match level {
            domain::enums::QuoteSignatureLevelEnum::Simple => v1::QuoteSignatureLevel::Simple,
            domain::enums::QuoteSignatureLevelEnum::Verified => v1::QuoteSignatureLevel::Verified,
        }
    }

    pub fn signature_level_to_domain(
        level: v1::QuoteSignatureLevel,
    ) -> domain::enums::QuoteSignatureLevelEnum {
        match level {
            v1::QuoteSignatureLevel::Simple => domain::enums::QuoteSignatureLevelEnum::Simple,
            v1::QuoteSignatureLevel::Verified => domain::enums::QuoteSignatureLevelEnum::Verified,
        }
    }

    pub fn approval_status_to_proto(
        status: domain::enums::QuoteApprovalStatusEnum,
    ) -> v1::QuoteApprovalStatus {
        use domain::enums::QuoteApprovalStatusEnum as S;
        match status {
            S::NotRequired => v1::QuoteApprovalStatus::ApprovalNotRequired,
            S::Pending => v1::QuoteApprovalStatus::ApprovalPending,
            S::Approved => v1::QuoteApprovalStatus::ApprovalApproved,
            S::Rejected => v1::QuoteApprovalStatus::ApprovalRejected,
        }
    }

    pub fn display_options_to_proto(
        options: &domain::quotes::QuoteDisplayOptions,
    ) -> v1::QuoteDisplayOptions {
        v1::QuoteDisplayOptions {
            hide_zero_lines: options.hide_zero_lines,
            show_list_price: options.show_list_price,
        }
    }

    pub fn display_options_to_domain(
        options: Option<v1::QuoteDisplayOptions>,
    ) -> domain::quotes::QuoteDisplayOptions {
        options
            .map(|o| domain::quotes::QuoteDisplayOptions {
                hide_zero_lines: o.hide_zero_lines,
                show_list_price: o.show_list_price,
            })
            .unwrap_or_default()
    }

    pub fn section_to_proto(section: &domain::quotes::QuoteSection) -> v1::QuoteSection {
        v1::QuoteSection {
            title: section.title.clone(),
            body: section.body.clone(),
            placement: match section.placement {
                domain::quotes::QuoteSectionPlacement::BeforePricing => {
                    v1::QuoteSectionPlacement::BeforePricing
                }
                domain::quotes::QuoteSectionPlacement::AfterPricing => {
                    v1::QuoteSectionPlacement::AfterPricing
                }
            } as i32,
        }
    }

    pub fn section_to_domain(section: v1::QuoteSection) -> domain::quotes::QuoteSection {
        domain::quotes::QuoteSection {
            placement: match section.placement() {
                v1::QuoteSectionPlacement::BeforePricing => {
                    domain::quotes::QuoteSectionPlacement::BeforePricing
                }
                v1::QuoteSectionPlacement::AfterPricing => {
                    domain::quotes::QuoteSectionPlacement::AfterPricing
                }
            },
            title: section.title,
            body: section.body,
        }
    }

    pub fn countersigner_to_proto(
        countersigner: &domain::quotes::QuoteCountersigner,
    ) -> v1::QuoteCountersigner {
        v1::QuoteCountersigner {
            name: countersigner.name.clone(),
            email: countersigner.email.clone(),
            title: countersigner.title.clone(),
            automatic: countersigner.automatic,
        }
    }

    pub fn countersigner_to_domain(
        countersigner: v1::QuoteCountersigner,
    ) -> domain::quotes::QuoteCountersigner {
        domain::quotes::QuoteCountersigner {
            name: countersigner.name,
            email: countersigner.email,
            title: countersigner.title.filter(|t| !t.trim().is_empty()),
            automatic: countersigner.automatic,
        }
    }

    pub fn draft_to_proto(draft: &domain::quote_drafts::QuoteDraft) -> v1::QuoteDraft {
        v1::QuoteDraft {
            id: draft.id.as_proto(),
            quote_id: draft.quote_id.map(|id| id.as_proto()),
            created_by: draft.created_by.as_proto(),
            updated_by: draft.updated_by.map(|id| id.as_proto()),
            label: draft.label.clone(),
            payload: draft.payload.to_string(),
            payload_version: draft.payload_version,
            version: draft.version,
            created_at: draft.created_at.as_proto(),
            updated_at: draft.updated_at.as_proto(),
        }
    }

    pub fn revision_to_proto(revision: &domain::quote_signing::QuoteRevision) -> v1::QuoteRevision {
        v1::QuoteRevision {
            revision: revision.revision,
            document_hash: revision.document_hash.clone(),
            document: revision
                .document
                .as_ref()
                .and_then(|d| serde_json::to_string(d).ok())
                .unwrap_or_default(),
            created_by: revision.created_by.clone(),
            created_at: revision.created_at.as_proto(),
            superseded_at: revision.superseded_at.as_proto(),
        }
    }

    pub fn evidence_to_proto(
        event: &domain::quote_signing::QuoteEvidenceEvent,
    ) -> v1::QuoteEvidenceEvent {
        v1::QuoteEvidenceEvent {
            seq: event.seq,
            kind: event.kind.clone(),
            actor_email: event.actor_email.clone(),
            ip_address: event.ip_address.clone(),
            user_agent: event.user_agent.clone(),
            revision: event.revision,
            document_hash: event.document_hash.clone(),
            details: event.details.to_string(),
            occurred_at: event.occurred_at.as_proto(),
            hash: event.hash.clone(),
        }
    }

    pub fn approval_rule_to_proto(
        rule: &domain::quote_approvals::QuoteApprovalRule,
    ) -> v1::QuoteApprovalRule {
        v1::QuoteApprovalRule {
            id: rule.id.as_proto(),
            name: rule.name.clone(),
            min_discount_percent: rule.min_discount_percent.map(|d| d.normalize().to_string()),
            max_net_terms_days: rule.max_net_terms_days,
            max_trial_days: rule.max_trial_days,
            always: rule.always,
            approver_user_ids: rule
                .approver_user_ids
                .iter()
                .map(|id| id.as_proto())
                .collect(),
        }
    }

    pub fn approval_to_proto(
        approval: &domain::quote_approvals::QuoteApproval,
    ) -> v1::QuoteApproval {
        v1::QuoteApproval {
            id: approval.id.as_proto(),
            quote_id: approval.quote_id.as_proto(),
            revision: approval.revision,
            status: approval_status_to_proto(approval.status) as i32,
            rule_ids: approval.rule_ids.iter().map(|id| id.as_proto()).collect(),
            requested_by: approval.requested_by.map(|id| id.as_proto()),
            requested_at: approval.requested_at.as_proto(),
            request_note: approval.request_note.clone(),
            decided_by: approval.decided_by.map(|id| id.as_proto()),
            decided_at: approval.decided_at.as_proto(),
            decision_note: approval.decision_note.clone(),
        }
    }

    pub fn detailed_quote_domain_to_proto(
        detailed_quote: &domain::quotes::DetailedQuote,
    ) -> DetailedQuote {
        let quote = &detailed_quote.quote;
        let components = &detailed_quote.components;
        let add_ons = &detailed_quote.add_ons;
        let coupons = &detailed_quote.coupons;
        let signatures = &detailed_quote.signatures;

        let customer_server: ServerCustomerWrapper =
            detailed_quote.customer.clone().try_into().unwrap();

        DetailedQuote {
            quote: Some(quote_to_proto(
                quote,
                Some(detailed_quote.customer.name.clone()),
                false,
            )),
            invoicing_entity: Some(
                crate::invoicingentities::mapping::invoicing_entities::domain_to_proto(
                    detailed_quote.invoicing_entity.clone(),
                ),
            ),
            customer: Some(customer_server.0),
            components: components
                .iter()
                .map(|c| quote_component_to_proto(c, &quote.currency))
                .collect(),
            add_ons: add_ons.iter().map(quote_add_on_to_proto).collect(),
            coupons: coupons.iter().map(quote_coupon_to_proto).collect(),
            coupon_terms: detailed_quote
                .coupon_terms()
                .iter()
                .map(coupon_terms_to_proto)
                .collect(),
            signatures: signatures.iter().map(quote_signature_to_proto).collect(),
            minimum_commitment: detailed_quote
                .minimum
                .as_ref()
                .map(crate::plans::mapping::plans::quote_minimum_to_proto),
            quote_email_enabled: None,
            engagement: None,
            totals: None,
            signature_codes_unavailable: false,
            pdf_url: None,
        }
    }

    pub fn totals_to_proto(totals: &domain::quotes::QuoteTotals) -> v1::QuoteTotals {
        v1::QuoteTotals {
            currency: totals.currency.clone(),
            recurring: totals
                .recurring
                .iter()
                .map(|r| v1::QuoteRecurringTotal {
                    period: subscription_fee_billing_period_to_grpc(r.period) as i32,
                    amount: r.amount.to_string(),
                    list_amount: r.list_amount.to_string(),
                })
                .collect(),
            has_usage: totals.has_usage,
            first_invoice_subtotal: totals.first_invoice_subtotal.map(|d| d.to_string()),
        }
    }

    pub fn engagement_to_proto(
        engagement: &[domain::quote_signing::QuoteRecipientEngagement],
        sessions: &[domain::quote_signing::QuoteViewSession],
    ) -> v1::QuoteEngagement {
        v1::QuoteEngagement {
            recipients: engagement
                .iter()
                .map(|e| v1::QuoteRecipientEngagement {
                    email: e.email.clone(),
                    view_count: e.view_count.max(0) as u32,
                    first_viewed_at: e.first_viewed_at.as_proto(),
                    last_viewed_at: e.last_viewed_at.as_proto(),
                    last_viewed_revision: e.last_viewed_revision.max(0) as u32,
                })
                .collect(),
            sessions: sessions
                .iter()
                .map(|s| v1::QuoteViewSession {
                    email: s.email.clone(),
                    revision: s.revision.max(0) as u32,
                    viewed_at: s.viewed_at.as_proto(),
                    last_seen_at: s.last_seen_at.as_proto(),
                    ip_address: s.ip_address.clone(),
                    device: s
                        .user_agent
                        .as_deref()
                        .map(meteroid_app::security_context::device_label),
                })
                .collect(),
        }
    }

    pub fn quote_add_on_to_proto(
        add_on: &domain::quotes::QuoteAddOn,
    ) -> meteroid_grpc::meteroid::api::quotes::v1::QuoteAddOn {
        use crate::subscriptions::mapping::price_components::{
            subscription_fee_billing_period_to_grpc, subscription_fee_to_grpc,
        };
        use meteroid_grpc::meteroid::api::quotes::v1::QuoteAddOn;

        QuoteAddOn {
            id: add_on.id.as_proto(),
            quote_id: add_on.quote_id.as_proto(),
            add_on_id: add_on.add_on_id.as_proto(),
            name: add_on.name.clone(),
            period: subscription_fee_billing_period_to_grpc(add_on.period) as i32,
            fee: Some(subscription_fee_to_grpc(
                &add_on.fee,
                add_on.period.as_billing_period_opt().unwrap_or_default(),
            )),
            quantity: add_on.quantity.max(0) as u32,
            description: add_on.description.clone(),
            sort_order: add_on.sort_order,
            discount_percent: add_on.discount_percent.map(|d| d.normalize().to_string()),
            list_fee: add_on.list_fee.as_ref().map(|f| {
                subscription_fee_to_grpc(
                    f,
                    add_on.period.as_billing_period_opt().unwrap_or_default(),
                )
            }),
        }
    }

    pub fn quote_coupon_to_proto(
        coupon: &domain::quotes::QuoteCoupon,
    ) -> meteroid_grpc::meteroid::api::quotes::v1::QuoteCoupon {
        use meteroid_grpc::meteroid::api::quotes::v1::QuoteCoupon;

        QuoteCoupon {
            id: coupon.id.as_proto(),
            quote_id: coupon.quote_id.as_proto(),
            coupon_id: coupon.coupon_id.as_proto(),
        }
    }

    pub fn coupon_terms_to_proto(
        terms: &domain::quote_document::QuoteDocumentCoupon,
    ) -> v1::QuoteCouponTerms {
        v1::QuoteCouponTerms {
            code: terms.code.clone(),
            description: terms.description.clone(),
            discount: Some(crate::coupons::mapping::coupons::discount::to_server(
                &terms.discount,
            )),
            recurring_value: terms.recurring_value.map(|v| v.max(0) as u32),
        }
    }

    pub fn recipient_details_to_domain(
        recipient: RecipientDetails,
    ) -> domain::quotes::RecipientDetails {
        domain::quotes::RecipientDetails {
            name: recipient.name,
            email: recipient.email,
        }
    }

    pub fn activation_condition_to_domain(
        condition: ActivationCondition,
    ) -> domain::enums::SubscriptionActivationCondition {
        match condition {
            ActivationCondition::OnStart => domain::enums::SubscriptionActivationCondition::OnStart,
            ActivationCondition::OnCheckout => {
                domain::enums::SubscriptionActivationCondition::OnCheckout
            }
            ActivationCondition::Manual => domain::enums::SubscriptionActivationCondition::Manual,
        }
    }

    pub fn activation_condition_to_proto(
        condition: domain::enums::SubscriptionActivationCondition,
    ) -> ActivationCondition {
        match condition {
            domain::enums::SubscriptionActivationCondition::OnStart => ActivationCondition::OnStart,
            domain::enums::SubscriptionActivationCondition::OnCheckout => {
                ActivationCondition::OnCheckout
            }
            domain::enums::SubscriptionActivationCondition::Manual => ActivationCondition::Manual,
        }
    }

    pub fn payment_methods_config_to_domain(
        config: Option<sub_proto::PaymentMethodsConfig>,
    ) -> Option<PaymentMethodsConfig> {
        use meteroid_store::domain::subscriptions::{OnlineMethodConfig, OnlineMethodsConfig};

        config.map(|proto_config| match proto_config.config {
            None => PaymentMethodsConfig::Online { config: None },
            Some(sub_proto::payment_methods_config::Config::Online(online)) => {
                let config = online.config.map(|cfg| OnlineMethodsConfig {
                    card: cfg.card.map(|m| OnlineMethodConfig { enabled: m.enabled }),
                    direct_debit: cfg
                        .direct_debit
                        .map(|m| OnlineMethodConfig { enabled: m.enabled }),
                });
                PaymentMethodsConfig::Online { config }
            }
            Some(sub_proto::payment_methods_config::Config::BankTransfer(_)) => {
                PaymentMethodsConfig::BankTransfer { account_id: None }
            }
            Some(sub_proto::payment_methods_config::Config::External(_)) => {
                PaymentMethodsConfig::External
            }
        })
    }
}
