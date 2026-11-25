//! What a response names of another resource (ADR-0013 §3): an object of its id and its human
//! key, one schema per resource wherever it is named (`CustomerRef`, `SubscriptionRef`,
//! `PlanRef`…). A response reads the keys of everything it names in one round
//! ([`Refs::load`], a list for its whole page): a key not found (another tenant's, deleted)
//! reads as the id.
use crate::v2::problem::Problem;
use common_domain::ids::{
    AddOnId, BillableMetricId, CouponId, CreditNoteId, CreditUnitId, CustomerId, FeatureId,
    InvoiceId, PlanId, ProductFamilyId, ProductId, SubscriptionId, TenantId,
};
use meteroid_store::Store;
use meteroid_store::repositories::CatalogCodesInterface;
pub use meteroid_store::repositories::catalog_codes::ReferenceIds;
pub use meteroid_store::repositories::catalog_codes::{CustomerKeys, ReferenceKeys};
use serde::Serialize;
use utoipa::ToSchema;

/// A customer named by another resource.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct CustomerRef {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CustomerId,
    /// The integrator's own id of the customer; name it as `ext:<alias>` in requests.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub name: String,
}

/// A subscription named by another resource.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct SubscriptionRef {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: SubscriptionId,
    /// The integrator's own id of the subscription; paths take it in place of the id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
}

/// A plan named by another resource: a subscription's and a checkout's at the version and the
/// option bought, a pricing card's as the plan alone.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq)]
pub struct PlanRef {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: PlanId,
    pub code: String,
    /// The version number; none where the plan is named alone (a pricing card).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    /// The option, by alias; none for a subscription older than options, or the plan named
    /// alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
}

macro_rules! coded_ref {
    ($name:ident, $id:ty, $what:literal) => {
        #[doc = concat!("A ", $what, " named by another resource.")]
        #[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
            pub id: $id,
            pub code: String,
        }
    };
}

coded_ref!(ProductRef, ProductId, "product");
coded_ref!(AddOnRef, AddOnId, "add-on");
coded_ref!(ProductFamilyRef, ProductFamilyId, "product family");
coded_ref!(MetricRef, BillableMetricId, "metric");
coded_ref!(CreditUnitRef, CreditUnitId, "credit unit");
coded_ref!(CouponRef, CouponId, "coupon");
coded_ref!(FeatureRef, FeatureId, "feature");

/// An invoice named by another resource.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct InvoiceRef {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: InvoiceId,
    /// Its invoice number.
    pub number: String,
}

/// A credit note named by another resource.
#[derive(Serialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct CreditNoteRef {
    #[serde(serialize_with = "common_domain::ids::string_serde::serialize")]
    pub id: CreditNoteId,
    /// Its credit note number.
    pub number: String,
}

/// The keys of what a response names, read once ([`Refs::load`]).
#[derive(Debug, Clone, Default)]
pub struct Refs(ReferenceKeys);

impl Refs {
    /// Keys read beside the store (a module's own connection).
    pub fn from_keys(keys: ReferenceKeys) -> Self {
        Refs(keys)
    }

    /// The keys of everything `ids` names, one query per kind named.
    pub async fn load(
        store: &Store,
        tenant_id: TenantId,
        ids: &ReferenceIds,
    ) -> Result<Self, Problem> {
        Ok(Refs(store.reference_keys(tenant_id, ids).await?))
    }

    pub fn customer(&self, id: CustomerId) -> CustomerRef {
        match self.0.customers.get(&id) {
            Some(keys) => CustomerRef {
                id,
                alias: keys.alias.clone(),
                name: keys.name.clone(),
            },
            None => CustomerRef {
                id,
                alias: None,
                name: id.to_string(),
            },
        }
    }

    /// The customer of `id` as it reads now, else (not read) with the alias and name given, as
    /// a document printed them.
    pub fn customer_or(&self, id: CustomerId, alias: Option<&str>, name: &str) -> CustomerRef {
        match self.0.customers.contains_key(&id) {
            true => self.customer(id),
            false => CustomerRef {
                id,
                alias: alias.map(str::to_owned),
                name: name.to_owned(),
            },
        }
    }

    pub fn subscription(&self, id: SubscriptionId) -> SubscriptionRef {
        SubscriptionRef {
            id,
            external_id: self.0.subscriptions.get(&id).cloned().flatten(),
        }
    }

    pub fn plan(&self, id: PlanId, version: Option<i32>, option: Option<String>) -> PlanRef {
        PlanRef {
            id,
            code: self
                .0
                .plans
                .get(&id)
                .cloned()
                .unwrap_or_else(|| id.to_string()),
            version,
            option,
        }
    }

    /// Holds the code of a plan read beside.
    pub fn know_plan(&mut self, id: PlanId, code: String) {
        self.0.plans.insert(id, code);
    }

    /// The coupon of `id`; none when it is not the tenant's (deleted since it was named).
    pub fn found_coupon(&self, id: CouponId) -> Option<CouponRef> {
        self.0.coupons.get(&id).map(|code| CouponRef {
            id,
            code: code.clone(),
        })
    }

    pub fn product(&self, id: ProductId) -> ProductRef {
        ProductRef {
            id,
            code: key(&self.0.products, id),
        }
    }

    pub fn add_on(&self, id: AddOnId) -> AddOnRef {
        AddOnRef {
            id,
            code: key(&self.0.add_ons, id),
        }
    }

    pub fn product_family(&self, id: ProductFamilyId) -> ProductFamilyRef {
        ProductFamilyRef {
            id,
            code: key(&self.0.product_families, id),
        }
    }

    pub fn metric(&self, id: BillableMetricId) -> MetricRef {
        MetricRef {
            id,
            code: key(&self.0.metrics, id),
        }
    }

    pub fn credit_unit(&self, id: CreditUnitId) -> CreditUnitRef {
        CreditUnitRef {
            id,
            code: key(&self.0.credit_units, id),
        }
    }

    /// The credit unit of a code, as a wallet or a grant names it; none when it is not the
    /// tenant's.
    pub fn credit_unit_coded(&self, code: &str) -> Option<CreditUnitRef> {
        self.0.credit_unit_ids.get(code).map(|id| CreditUnitRef {
            id: *id,
            code: code.to_owned(),
        })
    }

    pub fn coupon(&self, id: CouponId) -> CouponRef {
        CouponRef {
            id,
            code: key(&self.0.coupons, id),
        }
    }

    pub fn feature(&self, id: FeatureId) -> FeatureRef {
        FeatureRef {
            id,
            code: key(&self.0.features, id),
        }
    }

    pub fn invoice(&self, id: InvoiceId) -> InvoiceRef {
        InvoiceRef {
            id,
            number: key(&self.0.invoices, id),
        }
    }

    pub fn credit_note(&self, id: CreditNoteId) -> CreditNoteRef {
        CreditNoteRef {
            id,
            number: key(&self.0.credit_notes, id),
        }
    }
}

fn key<ID: std::hash::Hash + Eq + std::fmt::Display>(
    keys: &std::collections::HashMap<ID, String>,
    id: ID,
) -> String {
    keys.get(&id).cloned().unwrap_or_else(|| id.to_string())
}
