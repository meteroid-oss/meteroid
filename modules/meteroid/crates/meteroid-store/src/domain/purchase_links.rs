//! Purchase links: public, reusable links to buy a plan. Each purchase started from one
//! creates a customer and a SelfServe checkout session (not to be confused with the
//! checkout session itself, which is single-use).

use chrono::NaiveDateTime;
use common_domain::ids::{
    CheckoutSessionId, CustomerId, PlanId, PurchaseLinkId, PurchaseLinkSessionId, SubscriptionId,
    TenantId,
};
use diesel_models::enums::{CustomerTypeEnum, TaxIdCollectionEnum};
use diesel_models::purchase_links::{PurchaseLinkRow, PurchaseLinkSessionRow};
use o2o::o2o;

use crate::domain::enums::CustomerType;
use crate::domain::{Plan, PlanVersion};

pub const PURCHASE_LINK_TOKEN_LENGTH: usize = 32;

/// An installation's id as it reports it: a UUID for Meteroid, any opaque string otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstanceId(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error(
    "an instance id has 1 to {} characters, without spaces",
    InstanceId::MAX_LEN
)]
pub struct InvalidInstanceId;

impl InstanceId {
    pub const MAX_LEN: usize = 255;

    /// Spaces are refused: they separate the fields of a signed usage file.
    pub fn parse(id: &str) -> Result<Self, InvalidInstanceId> {
        let valid = !id.is_empty()
            && id.chars().count() <= Self::MAX_LEN
            && !id.chars().any(|c| c.is_whitespace() || c.is_control());
        valid.then(|| Self(id.to_owned())).ok_or(InvalidInstanceId)
    }

    /// For an id read back from storage, validated when it was written.
    pub fn stored(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<uuid::Uuid> for InstanceId {
    fn from(id: uuid::Uuid) -> Self {
        Self(id.to_string())
    }
}

impl std::fmt::Display for InstanceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who may buy from a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AllowedBuyerTypes {
    #[default]
    All,
    CompaniesOnly,
    IndividualsOnly,
}

impl AllowedBuyerTypes {
    pub fn allows(self, customer_type: CustomerType) -> bool {
        match self {
            AllowedBuyerTypes::All => true,
            AllowedBuyerTypes::CompaniesOnly => customer_type == CustomerType::Company,
            AllowedBuyerTypes::IndividualsOnly => customer_type == CustomerType::Individual,
        }
    }

    pub fn from_row(only: Option<CustomerTypeEnum>) -> Self {
        match only {
            None => AllowedBuyerTypes::All,
            Some(CustomerTypeEnum::Company) => AllowedBuyerTypes::CompaniesOnly,
            Some(CustomerTypeEnum::Individual) => AllowedBuyerTypes::IndividualsOnly,
        }
    }

    pub fn to_row(self) -> Option<CustomerTypeEnum> {
        match self {
            AllowedBuyerTypes::All => None,
            AllowedBuyerTypes::CompaniesOnly => Some(CustomerTypeEnum::Company),
            AllowedBuyerTypes::IndividualsOnly => Some(CustomerTypeEnum::Individual),
        }
    }
}

/// Whether companies are asked for a VAT / tax ID; individuals never are.
#[derive(o2o, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[map_owned(TaxIdCollectionEnum)]
pub enum TaxIdCollection {
    Off,
    #[default]
    Optional,
    Required,
}

#[derive(Debug, Clone)]
pub struct PurchaseLink {
    pub id: PurchaseLinkId,
    pub tenant_id: TenantId,
    pub plan_id: PlanId,
    pub name: String,
    pub token: String,
    pub allow_coupon_codes: bool,
    pub listed: bool,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    /// When false, only the billing country is required from buyers.
    pub require_billing_address: bool,
    pub tax_id_collection: TaxIdCollection,
    pub allowed_buyer_types: AllowedBuyerTypes,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub archived_at: Option<NaiveDateTime>,
}

impl From<PurchaseLinkRow> for PurchaseLink {
    fn from(row: PurchaseLinkRow) -> Self {
        Self {
            id: row.id,
            tenant_id: row.tenant_id,
            plan_id: row.plan_id,
            name: row.name,
            token: row.token,
            allow_coupon_codes: row.allow_coupon_codes,
            listed: row.listed,
            success_url: row.success_url,
            cancel_url: row.cancel_url,
            require_billing_address: row.require_billing_address,
            tax_id_collection: row.tax_id_collection.into(),
            allowed_buyer_types: AllowedBuyerTypes::from_row(row.only_buyer_type),
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PurchaseLinkNew {
    pub tenant_id: TenantId,
    pub plan_id: PlanId,
    pub name: String,
    pub allow_coupon_codes: bool,
    pub listed: bool,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    pub require_billing_address: bool,
    pub tax_id_collection: TaxIdCollection,
    pub allowed_buyer_types: AllowedBuyerTypes,
}

#[derive(Debug, Clone)]
pub struct PurchaseLinkUpdate {
    pub name: String,
    pub allow_coupon_codes: bool,
    pub listed: bool,
    pub success_url: Option<String>,
    pub cancel_url: Option<String>,
    pub require_billing_address: bool,
    pub tax_id_collection: TaxIdCollection,
    pub allowed_buyer_types: AllowedBuyerTypes,
}

#[derive(Debug, Clone)]
pub struct PurchaseLinkWithStats {
    pub link: PurchaseLink,
    pub plan_name: String,
    pub started_count: u32,
    pub completed_count: u32,
}

/// A link that can be bought from right now, with the plan version a purchase subscribes to.
#[derive(Debug, Clone)]
pub struct LivePurchaseLink {
    pub link: PurchaseLink,
    pub plan: Plan,
    pub version: PlanVersion,
}

/// A link offered to licensed software (see `listed`).
#[derive(Debug, Clone)]
pub struct ListedPurchaseLink {
    pub id: PurchaseLinkId,
    pub name: String,
    pub plan_name: String,
    pub plan_description: Option<String>,
    pub token: String,
}

/// The installation that started a purchase, bound to the subscription it produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurchaseBinding {
    pub subscription_id: SubscriptionId,
    pub instance_id: InstanceId,
    pub instance_public_key: String,
}

#[derive(Debug, Clone)]
pub struct PurchaseLinkSessionNew {
    pub tenant_id: TenantId,
    pub purchase_link_id: PurchaseLinkId,
    pub customer_id: CustomerId,
    pub checkout_session_id: CheckoutSessionId,
    pub attempt_id: String,
    pub license_instance_id: Option<InstanceId>,
    pub license_instance_public_key: Option<String>,
}

/// Points an attempt whose checkout closed unpaid at a new checkout.
#[derive(Debug, Clone)]
pub struct PurchaseAttemptRestart {
    pub customer_id: CustomerId,
    pub checkout_session_id: CheckoutSessionId,
    pub license_instance_id: Option<InstanceId>,
    pub license_instance_public_key: Option<String>,
}

/// The attempt was recorded or moved concurrently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurchaseRecordConflict;

#[derive(Debug, Clone)]
pub struct PurchaseLinkSession {
    pub id: PurchaseLinkSessionId,
    pub tenant_id: TenantId,
    pub purchase_link_id: PurchaseLinkId,
    pub customer_id: CustomerId,
    pub checkout_session_id: Option<CheckoutSessionId>,
    pub attempt_id: String,
    pub license_instance_id: Option<InstanceId>,
    pub created_at: NaiveDateTime,
    pub cleaned_up_at: Option<NaiveDateTime>,
}

impl From<PurchaseLinkSessionRow> for PurchaseLinkSession {
    fn from(row: PurchaseLinkSessionRow) -> Self {
        Self {
            id: row.id,
            tenant_id: row.tenant_id,
            purchase_link_id: row.purchase_link_id,
            customer_id: row.customer_id,
            checkout_session_id: row.checkout_session_id,
            attempt_id: row.attempt_id,
            license_instance_id: row.license_instance_id.map(InstanceId::stored),
            created_at: row.created_at,
            cleaned_up_at: row.cleaned_up_at,
        }
    }
}

pub fn generate_purchase_link_token() -> String {
    use rand::RngExt;
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut rng = rand::rng();
    (0..PURCHASE_LINK_TOKEN_LENGTH)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_base62_and_distinct() {
        let a = generate_purchase_link_token();
        let b = generate_purchase_link_token();
        assert_eq!(a.len(), PURCHASE_LINK_TOKEN_LENGTH);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_ne!(a, b);
    }
}
