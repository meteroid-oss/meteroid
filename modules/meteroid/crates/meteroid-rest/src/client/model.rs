//! The client API's own resources, in API v2's conventions (ADR-0013 §3): snake_case, prefixed
//! ids, codes beside them, lists paged by cursor. Each is a projection of what v2 reads, built by
//! v2's rendering, so nothing only the merchant reads (custom properties, MRR, invoicing, tax)
//! can reach a browser through a shared struct.
use crate::v2::charges::Cadence;
use crate::v2::customers::model::CustomerKind;
use crate::v2::subscriptions::model::{CurrentPeriod, Dates, SubscriptionStatus, TrialState};
use common_domain::ids::{CustomerId, SubscriptionId, string_serde};
use serde::Serialize;
use utoipa::ToSchema;

/// The customer the token was minted for: who it is, as v2 reads a customer.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ClientCustomer {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: CustomerId,
    /// The merchant's own id of the customer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(rename = "type")]
    pub kind: CustomerKind,
    /// A company's name, or a person's full name.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
    /// The billing email.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// What the customer is billed in (ISO 4217).
    pub currency: String,
}

/// The plan a subscription is on, by code, with its name to show.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ClientPlanRef {
    pub code: String,
    pub name: String,
    pub version: i32,
    /// The option bought, by alias; none for a subscription older than options.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
}

/// A subscription of the customer, as v2's summary reads it (`GET
/// /api/v2/subscriptions/{id}/summary`), less what only the merchant reads: its MRR, invoicing,
/// custom properties, resource and external id.
#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct ClientSubscription {
    #[serde(serialize_with = "string_serde::serialize")]
    pub id: SubscriptionId,
    pub plan: ClientPlanRef,
    pub status: SubscriptionStatus,
    pub currency: String,
    /// How often it renews.
    pub period: Cadence,
    pub dates: Dates,
    pub current_period: CurrentPeriod,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial: Option<TrialState>,
}

crate::v2_list!(ClientSubscriptionList, ClientSubscription);
