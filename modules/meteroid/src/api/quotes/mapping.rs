pub mod quotes {
    use crate::api::shared::conversions::{AsProtoOpt, ProtoConv};

    use meteroid_grpc::meteroid::api::quotes::v1::{
        DetailedQuote, Quote, QuoteActivity, QuoteComponent, QuoteSignature, QuoteStatus,
        RecipientDetails,
    };
    use meteroid_grpc::meteroid::api::subscriptions::v1::ActivationCondition;
    use meteroid_store::domain;

    use crate::api::customers::mapping::customer::ServerCustomerWrapper;
    use crate::api::subscriptions::mapping::price_components::subscription_fee_to_grpc;

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

    pub(crate) fn quote_component_to_proto(
        component: &domain::quotes::QuotePriceComponent,
    ) -> QuoteComponent {
        QuoteComponent {
            id: component.id.as_proto(),
            name: component.name.clone(),
            price_component_id: component.price_component_id.map(|id| id.as_proto()),
            product_id: component.product_id.map(|id| id.as_proto()),
            period: component.period.clone() as i32,
            fee: Some(subscription_fee_to_grpc(&component.fee)),
            is_override: component.is_override,
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
        }
    }

    fn quote_activity_to_proto(activity: &domain::quotes::QuoteActivity) -> QuoteActivity {
        QuoteActivity {
            id: activity.id.as_proto(),
            quote_id: activity.quote_id.as_proto(),
            activity_type: activity.activity_type.clone(),
            description: activity.description.clone(),
            actor_type: activity.actor_type.clone(),
            actor_id: activity.actor_id.clone(),
            actor_name: activity.actor_name.clone(),
            created_at: activity.created_at.as_proto(),
            ip_address: activity.ip_address.clone(),
            user_agent: activity.user_agent.clone(),
        }
    }

    pub fn quote_to_proto(
        quote: &domain::quotes::Quote,
        customer_name: Option<String>,
        hide_internal_note: bool,
    ) -> Quote {
        Quote {
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
            billing_start_date: quote.billing_end_date.as_proto(),
            end_date: None, // TODO: Map end_date
            billing_day_anchor: quote.billing_day_anchor.map(|d| d as u32),
            activation_condition: activation_condition_to_proto(quote.activation_condition.clone())
                as i32,
            // Quote-specific fields
            valid_until: quote.valid_until.as_proto(),
            expires_at: quote.expires_at.as_proto(),
            accepted_at: quote.accepted_at.as_proto(),
            declined_at: quote.declined_at.as_proto(),
            internal_notes: if hide_internal_note {
                None
            } else {
                quote.internal_notes.clone()
            },
            cover_image: quote.cover_image.map(|id| id.as_proto()),
            overview: quote.overview.clone(),
            terms_and_services: quote.terms_and_services.clone(),
            attachments: quote
                .attachments
                .iter()
                .filter_map(|opt| opt.map(|id| id.as_proto()))
                .collect(),
            recipients: quote
                .recipients
                .iter()
                .map(recipient_details_to_proto)
                .collect(),
        }
    }

    pub fn detailed_quote_domain_to_proto(
        detailed_quote: &domain::quotes::DetailedQuote,
    ) -> DetailedQuote {
        let quote = &detailed_quote.quote;
        let components = &detailed_quote.components;
        let signatures = &detailed_quote.signatures;
        let activities = &detailed_quote.activities;

        let customer_server: ServerCustomerWrapper =
            detailed_quote.customer.clone().try_into().unwrap();

        DetailedQuote {
            quote: Some(quote_to_proto(
                quote,
                Some(detailed_quote.customer.name.clone()),
                false,
            )),
            invoicing_entity: Some(
                crate::api::invoicingentities::mapping::invoicing_entities::domain_to_proto(
                    detailed_quote.invoicing_entity.clone(),
                ),
            ),
            customer: Some(customer_server.0),
            components: components.iter().map(quote_component_to_proto).collect(),
            signatures: signatures.iter().map(quote_signature_to_proto).collect(),
            activities: activities.iter().map(quote_activity_to_proto).collect(),
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
}
