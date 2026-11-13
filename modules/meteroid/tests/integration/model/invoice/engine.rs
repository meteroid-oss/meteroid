//! Drives the engine through the invoice model's actions, in business time, on a Halyard tenant
//! (US invoicing entity, manual tax resolver, no invoicing grace) where each case builds its own
//! metric, two plans, customer and coupons; reads back what the model is compared with.
use std::str::FromStr;

use chrono::{DateTime, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, PlanVersionId, SubscriptionId};
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::enums::{
    BillingMetricAggregateEnum, BillingPeriodEnum, BillingType, CustomerType, InvoiceStatusEnum,
    PlanStatusEnum, PlanTypeEnum,
};
use meteroid_store::domain::price_components::{
    FeeType, PriceComponentNewInternal, PriceEntry, PriceInput, ProductRef, UsagePricingModel,
};
use meteroid_store::domain::prices::{extract_fee_structure, extract_pricing};
use meteroid_store::domain::subscription_coupons::{
    CreateSubscriptionCoupon, CreateSubscriptionCoupons,
};
use meteroid_store::domain::{
    Address, BillableMetricNew, CustomerNew, CustomerTaxRate, CustomerTaxStatus, FullPlanNew,
    Invoice, PaginationRequest, PlanNew, PlanVersionNewInternal,
};
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::plans::PlansInterfaceAuto;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::repositories::{
    CustomersInterface, InvoiceInterface, PlansInterface, TenantInterface,
};
use meteroid_store::services::InvoiceDelivery;
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use rust_decimal::Decimal;

use super::reference::{
    Action, Billing, Catalog, Discount, Model, PlanSpec, RATE_SCALE, fee_line, midnight, plan_name,
    units_line,
};
use crate::model::harness::Diff;
use crate::scenarios::kit::Halyard;

const PROPERTY: &str = "units";

pub struct Engine {
    pub h: Halyard,
    customer: Option<CustomerId>,
    sub: Option<SubscriptionId>,
    plans: Vec<PlanVersionId>,
    invoices: Vec<InvoiceId>,
}

/// An invoice as compared: status, [subtotal, discount, tax, total, due], lines billing
/// something as (name, subtotal), sorted.
type Seen = (InvoiceStatusEnum, [i64; 5], Vec<(String, i64)>);

fn err<E: std::fmt::Debug>(e: E) -> String {
    let text = format!("{e:?}");
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

/// `minor / 10^exponent` as an exact decimal.
fn major(minor: i64, exponent: u32) -> Decimal {
    Decimal::new(minor, exponent)
}

fn first_of_december() -> DateTime<Utc> {
    midnight(NaiveDate::from_ymd_opt(2024, 12, 1).expect("a day"))
}

impl Engine {
    pub async fn new() -> Self {
        let h = Halyard::new().await;
        h.env
            .skip_credit_invariants("the model runner checks them after every step");
        h.skip_corpus("a model case's documents follow from its seed");
        Self {
            h,
            customer: None,
            sub: None,
            plans: vec![],
            invoices: vec![],
        }
    }

    async fn due_events(&self) -> Result<(), String> {
        for _ in 0..4 {
            let processed = self
                .h
                .services()
                .get_and_process_due_events()
                .await
                .map_err(err)?;
            if processed == 0 {
                break;
            }
        }
        Ok(())
    }

    async fn drain(&self, passes: usize) {
        for _ in 0..passes {
            self.h.env.run_outbox_and_orchestration().await;
        }
    }

    pub async fn apply(&mut self, action: &Action) -> Result<(), String> {
        let outcome = self.run(action).await;
        self.drain(2).await;
        self.sync().await;
        outcome
    }

    async fn run(&mut self, action: &Action) -> Result<(), String> {
        let tenant = self.h.tenant;
        match action {
            Action::Start { day, catalog } => self.start(*day, catalog).await?,
            Action::Usage { at, units } => self.h.usage.record_with(
                self.customer.expect("started"),
                *at,
                &[(PROPERTY, units.to_string().as_str())],
            ),
            Action::Advance { at } => {
                self.h
                    .services()
                    .bill_subscription_period_at(
                        tenant,
                        self.sub.expect("started"),
                        *at,
                        InvoiceDelivery::Suppressed,
                    )
                    .await
                    .map_err(err)?;
                self.due_events().await?;
            }
            Action::ChangeAtEnd { to, .. } => {
                self.h
                    .services()
                    .schedule_plan_change(
                        Actor::System,
                        self.sub.expect("started"),
                        tenant,
                        self.plans[*to],
                        vec![],
                    )
                    .await
                    .map_err(err)?;
            }
            Action::CancelAtEnd { end, .. } => {
                self.h
                    .services()
                    .cancel_subscription(
                        Actor::System,
                        self.sub.expect("started"),
                        tenant,
                        None,
                        meteroid_store::repositories::subscriptions::CancellationEffectiveAt::Date(
                            *end,
                        ),
                    )
                    .await
                    .map_err(err)?;
            }
        }
        Ok(())
    }

    async fn start(&mut self, day: NaiveDate, catalog: &Catalog) -> Result<(), String> {
        let h = &self.h;
        let tenant = h.tenant;
        let currency = catalog.currency;
        let family = h
            .store()
            .get_plan_by_version_id(h.payg, tenant)
            .await
            .map_err(err)?
            .plan
            .product_family_id;
        let metric = h
            .store()
            .insert_billable_metric(
                Actor::System,
                BillableMetricNew {
                    name: "Model units".to_owned(),
                    description: None,
                    code: "model_units".to_owned(),
                    aggregation_type: BillingMetricAggregateEnum::Sum,
                    aggregation_key: Some(PROPERTY.to_owned()),
                    unit_conversion_factor: None,
                    unit_conversion_rounding: None,
                    segmentation_matrix: None,
                    filters: None,
                    usage_group_key: None,
                    unit_label: None,
                    tenant_id: tenant,
                    product_family_id: family,
                    product_id: None,
                },
            )
            .await
            .map_err(err)?;
        for (p, spec) in catalog.plans.iter().enumerate() {
            let PlanSpec { fee, billing, rate } = *spec;
            let component = |name: String, fee: FeeType| {
                let (fee_type, fee_structure) = extract_fee_structure(&fee);
                PriceComponentNewInternal {
                    name: name.clone(),
                    product_ref: ProductRef::New {
                        name,
                        fee_type,
                        fee_structure,
                    },
                    prices: extract_pricing(&fee)
                        .into_iter()
                        .map(|(cadence, pricing)| {
                            PriceEntry::New(PriceInput {
                                cadence,
                                currency: currency.code().to_owned(),
                                pricing,
                                credit_unit: None,
                            })
                        })
                        .collect(),
                }
            };
            let created = h
                .store()
                .insert_plan(
                    Actor::System,
                    FullPlanNew {
                        plan: PlanNew {
                            name: format!("Model plan {}", plan_name(p)),
                            description: None,
                            tenant_id: tenant,
                            product_family_id: family,
                            plan_type: PlanTypeEnum::Standard,
                            status: PlanStatusEnum::Active,
                        },
                        version: PlanVersionNewInternal {
                            is_draft_version: false,
                            period_start_day: None,
                            net_terms: 0,
                            currency: Some(currency.code().to_owned()),
                            billing_cycles: None,
                            trial: None,
                            entitlements: vec![],
                            tax_inclusive: false,
                            credit_exhausted_rule: None,
                        },
                        price_components: vec![
                            component(
                                fee_line(p),
                                FeeType::ExtraRecurring {
                                    unit_price: major(fee, currency.exponent()),
                                    quantity: 1,
                                    billing_type: match billing {
                                        Billing::Advance => BillingType::Advance,
                                        Billing::Arrears => BillingType::Arrears,
                                    },
                                    cadence: BillingPeriodEnum::Monthly,
                                },
                            ),
                            component(
                                units_line(p),
                                FeeType::Usage {
                                    metric_id: metric.id,
                                    pricing: UsagePricingModel::PerUnit {
                                        rate: Decimal::new(
                                            rate,
                                            currency.exponent() + RATE_SCALE.ilog10(),
                                        ),
                                    },
                                    cadence: BillingPeriodEnum::Monthly,
                                },
                            ),
                        ],
                    },
                )
                .await
                .map_err(err)?;
            self.plans.push(created.version.id);
        }
        let mut currencies = h
            .store()
            .list_tenant_currencies(tenant)
            .await
            .map_err(err)?;
        if !currencies.iter().any(|c| c == currency.code()) {
            currencies.push(currency.code().to_owned());
            h.store()
                .update_tenant_available_currencies(tenant, currencies)
                .await
                .map_err(err)?;
        }
        let customer = h
            .store()
            .insert_customer(
                Actor::System,
                CustomerNew {
                    name: "Model".to_owned(),
                    alias: None,
                    billing_email: Some("billing@model.test".to_owned()),
                    invoicing_emails: vec![],
                    phone: None,
                    currency: currency.code().to_owned(),
                    billing_address: Some(Address {
                        line1: Some("1 Market St".to_owned()),
                        line2: None,
                        city: Some("Springfield".to_owned()),
                        country: Some(
                            common_domain::country::CountryCode::from_str("US").map_err(err)?,
                        ),
                        state: Some("CA".to_owned()),
                        zip_code: Some("10001".to_owned()),
                    }),
                    shipping_address: None,
                    invoicing_entity_id: None,
                    force_created_date: Some(first_of_december().naive_utc()),
                    vat_number: None,
                    legal_number: None,
                    buyer_reference: None,
                    custom_taxes: catalog
                        .taxes
                        .iter()
                        .map(|t| CustomerTaxRate {
                            tax_code: t.code.to_owned(),
                            name: t.name.to_owned(),
                            rate: Decimal::new(t.ppm, 6),
                        })
                        .collect(),
                    tax_status: CustomerTaxStatus::Taxable,
                    exemption_reason: None,
                    connected_account_id: None,
                    custom_properties: serde_json::Value::Null,
                    preferred_locales: vec![],
                    customer_type: CustomerType::Company,
                    first_name: None,
                    last_name: None,
                },
                tenant,
            )
            .await
            .map_err(err)?
            .id;
        self.customer = Some(customer);
        let mut coupons = vec![];
        for (c, coupon) in catalog.coupons.iter().enumerate() {
            let discount = match coupon.discount {
                Discount::Percent(p) => CouponDiscount::Percentage(Decimal::from(p)),
                Discount::Fixed(amount) => CouponDiscount::Fixed {
                    amount: major(amount, currency.exponent()),
                    currency: currency.code().to_owned(),
                },
            };
            let created = h
                .services()
                .create_coupon_at(
                    Actor::System,
                    CouponNew {
                        code: format!("MODEL{c}"),
                        description: format!("Model coupon {c}"),
                        tenant_id: tenant,
                        discount,
                        expires_at: None,
                        redemption_limit: None,
                        recurring_value: coupon.cycles.map(|n| n as i32),
                        reusable: false,
                        plan_ids: vec![],
                    },
                    first_of_december(),
                )
                .await
                .map_err(err)?;
            coupons.push(CreateSubscriptionCoupon {
                coupon_id: created.id,
            });
        }
        let mut request = h.request(customer, self.plans[0], day);
        if !coupons.is_empty() {
            request.coupons = Some(CreateSubscriptionCoupons { coupons });
        }
        let (created, _) = h
            .services()
            .insert_subscription_started_at(
                Actor::System,
                request,
                tenant,
                midnight(day),
                InvoiceDelivery::Suppressed,
            )
            .await
            .map_err(err)?;
        self.sub = Some(created.id);
        Ok(())
    }

    async fn all_invoices(&self) -> Vec<Invoice> {
        let Some(customer) = self.customer else {
            return vec![];
        };
        let mut invoices: Vec<Invoice> = self
            .h
            .store()
            .list_invoices(
                self.h.tenant,
                Some(customer),
                None,
                None,
                None,
                None,
                false,
                None,
                vec![],
                Some("created_at.asc".to_string()),
                PaginationRequest {
                    page: 0,
                    per_page: Some(1_000),
                },
            )
            .await
            .expect("invoices")
            .items
            .into_iter()
            .map(|i| i.invoice)
            .collect();
        invoices.sort_by_key(|i| (i.created_at, i.id.as_uuid()));
        invoices
    }

    /// Maps the invoices issued since, in creation order; drafts and closed periods are not
    /// documents of the model.
    async fn sync(&mut self) {
        for invoice in self.all_invoices().await {
            if invoice.status == InvoiceStatusEnum::Closed
                || invoice.status == InvoiceStatusEnum::Draft
                || self.invoices.contains(&invoice.id)
            {
                continue;
            }
            self.invoices.push(invoice.id);
        }
    }

    /// The `n`-th mapped invoice.
    pub async fn invoice(&self, n: usize) -> Invoice {
        self.h.invoice(self.invoices[n]).await
    }

    fn seen(invoice: &Invoice) -> Seen {
        let mut lines: Vec<(String, i64)> = invoice
            .line_items
            .iter()
            .filter(|l| l.amount_subtotal != 0)
            .map(|l| (l.name.clone(), l.amount_subtotal))
            .collect();
        lines.sort();
        (
            invoice.status,
            [
                invoice.subtotal,
                invoice.discount,
                invoice.tax_amount,
                invoice.total,
                invoice.amount_due,
            ],
            lines,
        )
    }

    pub async fn compare(&self, model: &Model) -> Vec<Diff> {
        let all = self.all_invoices().await;
        let mut diffs = vec![];
        let unmapped: Vec<String> = all
            .iter()
            .filter(|i| !self.invoices.contains(&i.id) && i.status != InvoiceStatusEnum::Closed)
            .map(|i| format!("{:?} {} due {}", i.status, i.total, i.amount_due))
            .collect();
        if self.invoices.len() != model.invoices.len() || !unmapped.is_empty() {
            diffs.push((
                "invoice count".to_owned(),
                format!(
                    "expected {} invoices, the engine issued {} (unmapped: {unmapped:?})",
                    model.invoices.len(),
                    self.invoices.len(),
                ),
            ));
        }
        const FIGURES: [&str; 5] = ["subtotal", "discount", "tax", "total", "due"];
        for (i, (expected, id)) in model.invoices.iter().zip(&self.invoices).enumerate() {
            let invoice = all.iter().find(|x| x.id == *id).expect("a mapped invoice");
            let (status, figures, lines) = Self::seen(invoice);
            if status != InvoiceStatusEnum::Finalized {
                diffs.push((
                    "invoice status".to_owned(),
                    format!("INV-{}: expected finalized, engine {status:?}", i + 1),
                ));
            }
            if lines != expected.lines {
                diffs.push((
                    "invoice lines".to_owned(),
                    format!(
                        "INV-{}: lines expected {:?}, engine {lines:?}",
                        i + 1,
                        expected.lines
                    ),
                ));
            }
            let wanted = [
                expected.subtotal,
                expected.discount,
                expected.tax,
                expected.total,
                expected.due(),
            ];
            if let Some(k) = (0..FIGURES.len()).find(|&k| wanted[k] != figures[k]) {
                diffs.push((
                    format!("invoice {}", FIGURES[k]),
                    format!(
                        "INV-{}: {} expected {}, engine {} (expected {wanted:?}, engine {figures:?})",
                        i + 1,
                        FIGURES[k],
                        wanted[k],
                        figures[k]
                    ),
                ));
            }
        }
        diffs
    }

    async fn coupons(&self) -> Vec<String> {
        let Some(sub) = self.sub else {
            return vec![];
        };
        let details = self
            .h
            .store()
            .get_subscription_details(self.h.tenant, sub)
            .await
            .expect("the subscription");
        let mut coupons: Vec<String> = details
            .applied_coupons
            .iter()
            .map(|c| {
                format!(
                    "{} active {} count {:?} amount {:?}",
                    c.coupon.code,
                    c.applied_coupon.is_active,
                    c.applied_coupon.applied_count,
                    c.applied_coupon.applied_amount
                )
            })
            .collect();
        coupons.sort();
        coupons
    }

    pub async fn fingerprint(&self) -> Vec<String> {
        let mut seen: Vec<String> = self
            .all_invoices()
            .await
            .iter()
            .map(|i| {
                format!(
                    "{} {:?} {:?} {:?}",
                    i.id,
                    i.payment_status,
                    Self::seen(i),
                    i.coupons.iter().map(|c| c.value).collect::<Vec<_>>()
                )
            })
            .collect();
        seen.extend(self.coupons().await);
        seen
    }

    /// The scheduled events and the period's billing a second time: neither may write.
    pub async fn rerun_workers(&self, action: &Action) -> Result<(), String> {
        let h = &self.h;
        h.services()
            .settle_missed_plan_change_endings(100)
            .await
            .map_err(err)?;
        self.due_events().await?;
        if let (Action::Advance { at }, Some(sub)) = (action, self.sub)
            && let Ok(Some(invoice)) = h
                .services()
                .bill_subscription_period_at(h.tenant, sub, *at, InvoiceDelivery::Suppressed)
                .await
        {
            return Err(format!(
                "billing the period again at {at} returned invoice {}",
                invoice.invoice.id
            ));
        }
        Ok(())
    }

    /// Every invoice's lines as stored, and the coupons' state.
    pub async fn dump(&self) -> Vec<String> {
        let mut lines = vec![];
        for invoice in self.all_invoices().await {
            let n = self
                .invoices
                .iter()
                .position(|id| *id == invoice.id)
                .map_or("unmapped".to_owned(), |i| format!("INV-{}", i + 1));
            lines.push(format!(
                "document: {n} {:?} {:?} dated {} {} coupons {:?}",
                invoice.invoice_type,
                invoice.status,
                invoice.invoice_date,
                invoice.currency,
                invoice
                    .coupons
                    .iter()
                    .map(|c| (c.code.clone(), c.value))
                    .collect::<Vec<_>>()
            ));
            for l in &invoice.line_items {
                lines.push(format!(
                    "document:   {} {}..{} qty {:?} x {:?} = {}, taxable {}, tax {}",
                    l.name,
                    l.start_date,
                    l.end_date,
                    l.quantity,
                    l.unit_price,
                    l.amount_subtotal,
                    l.taxable_amount,
                    l.tax_amount
                ));
            }
        }
        lines.extend(
            self.coupons()
                .await
                .into_iter()
                .map(|c| format!("coupon: {c}")),
        );
        lines
    }

    pub async fn invariants(&self) -> Vec<String> {
        let mut conn = self.h.env.conn().await;
        crate::helpers::invariants::credit_invariant_violations(&mut conn, self.h.tenant).await
    }
}
