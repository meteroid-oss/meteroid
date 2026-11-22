//! Drives the engine through the lifecycle model's actions on a Halyard tenant (US invoicing
//! entity, no invoicing grace), its clock moved to each action's instant so previews and due
//! events read the present the action happens at; reads back what the model is checked against.
use chrono::{DateTime, Duration, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, BaseId, CustomerId, InvoiceId, PlanVersionId, PriceComponentId, SubscriptionId,
};
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::enums::{
    BillingMetricAggregateEnum, BillingPeriodEnum, BillingType, CreditNoteOrigin, CreditNoteStatus,
    FeeTypeEnum, InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType, PlanStatusEnum,
    PlanTypeEnum, SubscriptionActivationCondition,
};
use meteroid_store::domain::invoicing_entities::InvoicingEntityPatch;
use meteroid_store::domain::plan_minimums::CreateMinimumScope;
use meteroid_store::domain::plan_version_add_ons::PlanVersionAddOnNew;
use meteroid_store::domain::plans::PlanTrial;
use meteroid_store::domain::price_components::{
    CapacityThreshold, DowngradePolicy, FeeType, PriceComponentNewInternal, PriceEntry, PriceInput,
    ProductRef, TermRate, UpgradePolicy, UsagePricingModel,
};
use meteroid_store::domain::prices::{
    FeeStructure, Pricing, extract_fee_structure, extract_pricing,
};
use meteroid_store::domain::slot_transactions::SlotUpgradeBillingMode;
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddExtraComponent, AddOnChanges, ComponentChanges, EditComponent, EditSubscriptionAddOn,
    SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::{ChangeDocument, PlanChangeMode};
use meteroid_store::domain::subscription_components::{
    ComponentParameterization, ComponentParameters, CreateSubscriptionComponents,
};
use meteroid_store::domain::subscription_coupons::{
    CreateSubscriptionCoupon, CreateSubscriptionCoupons,
};
use meteroid_store::domain::{
    BillableMetricNew, CreditNote, FullPlanNew, Invoice, PaginationRequest, PlanNew,
    PlanVersionNewInternal,
};
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::plan_version_add_ons::PlanVersionAddOnInterface;
use meteroid_store::repositories::plans::PlansInterfaceAuto;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};
use meteroid_store::repositories::{CreditNoteInterface, InvoiceInterface, PlansInterface};
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use meteroid_store::services::{InvoiceDelivery, IssueAt};
use rust_decimal::Decimal;

use super::reference::{
    Action, Catalog, CouponSpec, Model, Off, PlanSpec, fee_line, midnight, plan_name, rounded,
    seats_line, units_line,
};
use crate::model::harness::{Diff, race, timed};
use crate::scenarios::kit::{Halyard, Tax};

const PROPERTY: &str = "units";
const MINIMUM_LINE: &str = "Minimum commitment";

pub struct Engine {
    pub h: Halyard,
    customer: Option<CustomerId>,
    sub: Option<SubscriptionId>,
    plans: Vec<PlanVersionId>,
    /// Each plan's seats component.
    seats: Vec<PriceComponentId>,
    /// Plan A's capacity component.
    capacity: Option<PriceComponentId>,
    add_on: Option<AddOnId>,
    /// The customer's second subscription, consolidated with this one.
    companion: Option<SubscriptionId>,
    /// What an action found wrong as it ran, reported by the next comparison.
    pending: Vec<Diff>,
    one_time: usize,
    /// When each draft issued by hand began to be issued: it is dated the moment asked, the
    /// notes written before it by the clock.
    issuing: std::collections::HashMap<InvoiceId, chrono::NaiveDateTime>,
}

fn err<E: std::fmt::Debug>(e: E) -> String {
    let text = format!("{e:?}");
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

fn cents(minor: i64) -> Decimal {
    Decimal::new(minor, 2)
}

fn cadence(months: u32) -> BillingPeriodEnum {
    if months == 12 {
        BillingPeriodEnum::Annual
    } else {
        BillingPeriodEnum::Monthly
    }
}

/// The present as the engine reads it: just after `at`. Cases run one at a time.
fn clock(at: DateTime<Utc>) {
    common_clock::test::set(at + Duration::minutes(1));
}

fn issued(invoice: &Invoice) -> bool {
    !matches!(
        invoice.status,
        InvoiceStatusEnum::Void | InvoiceStatusEnum::Closed
    )
}

impl Engine {
    pub async fn new() -> Self {
        clock(midnight(
            NaiveDate::from_ymd_opt(2024, 12, 1).expect("a day"),
        ));
        let h = Halyard::new().await;
        h.env
            .skip_credit_invariants("the model runner checks them after every step");
        h.skip_corpus("a model case's documents follow from its seed");
        Self {
            h,
            customer: None,
            sub: None,
            plans: vec![],
            seats: vec![],
            capacity: None,
            add_on: None,
            companion: None,
            pending: vec![],
            one_time: 0,
            issuing: std::collections::HashMap::new(),
        }
    }

    async fn settle(&self) -> Result<(), String> {
        let run = || async { self.h.services().run_invoice_deadlines().await.map_err(err) };
        // The controller renews the companion one period a run: it runs until it finds no work.
        let rounds = if self.companion.is_some() { 40 } else { 1 };
        for _ in 0..rounds {
            for _ in 0..4 {
                let (a, b) = race(run(), run()).await?;
                if a.unwrap_or(0) + b.unwrap_or(0) == 0 {
                    break;
                }
            }
            let done = self
                .h
                .services()
                .run_billing_controller(100)
                .await
                .map_err(err)?;
            if done.customers == 0 {
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

    pub async fn apply(&mut self, action: &Action, model: &Model) -> Result<(), String> {
        self.pending.clear();
        clock(action.at());
        let outcome = timed("apply: store calls", self.run(action, model)).await;
        let settled = timed("apply: controller passes", self.settle()).await;
        timed("apply: outbox and orchestration", self.drain(2)).await;
        outcome.and(settled)
    }

    fn sub(&self) -> SubscriptionId {
        self.sub.expect("started")
    }

    async fn run(&mut self, action: &Action, model: &Model) -> Result<(), String> {
        let tenant = self.h.tenant;
        // The companion ends with the subscription, the same way.
        let ending = match action {
            Action::CancelNow { at } => Some(CancellationEffectiveAt::Date(at.date_naive())),
            Action::CancelImmediately { .. } => Some(CancellationEffectiveAt::Immediate),
            Action::CancelBackdated { day, .. } => Some(CancellationEffectiveAt::Date(*day)),
            Action::CancelAtEnd { end, .. } => Some(CancellationEffectiveAt::Date(*end)),
            _ => None,
        };
        if let (Some(ending), Some(companion)) = (ending, self.companion) {
            self.h
                .services()
                .cancel_subscription(Actor::System, companion, tenant, None, ending)
                .await
                .map_err(|e| format!("the companion's cancellation: {}", err(e)))?;
        }
        match action {
            Action::Start { day, catalog } => self.start(*day, catalog).await?,
            Action::Usage { at, units } => self.h.usage.record_with(
                self.customer.expect("started"),
                *at,
                &[(PROPERTY, units.to_string().as_str())],
            ),
            // Consolidated renewals are the billing controller's, at the present (no past moment).
            Action::Advance { .. } if self.companion.is_some() => {}
            Action::Advance { at } => {
                let sub = self.sub();
                let run = || async {
                    self.h
                        .services()
                        .bill_subscription_period_at(tenant, sub, *at, InvoiceDelivery::Suppressed)
                        .await
                        .map_err(err)
                };
                race(run(), run()).await?;
            }
            Action::AmendNow {
                at,
                plan,
                price,
                months,
                rate,
            } => {
                let day = at.date_naive();
                let moved = *months != model.months();
                let fixed = Pricing::ExtraRecurring {
                    unit_price: cents(*price),
                    quantity: 1,
                };
                let mut edited = vec![self.edit(&fee_line(*plan), day, fixed, *months).await?];
                if moved {
                    edited.push(
                        self.edit(
                            &units_line(*plan),
                            day,
                            Pricing::Usage(UsagePricingModel::PerUnit { rate: cents(*rate) }),
                            *months,
                        )
                        .await?,
                    );
                }
                let amendment = SubscriptionAmendment {
                    apply_mode: PlanChangeMode::Immediate,
                    component_changes: ComponentChanges {
                        edited,
                        added: vec![],
                        removed: vec![],
                    },
                    add_on_changes: AddOnChanges::default(),
                };
                self.amend_now(amendment, day).await?;
            }
            Action::AmendAtEnd {
                at,
                plan,
                price,
                months,
            } => {
                let fixed = Pricing::ExtraRecurring {
                    unit_price: cents(*price),
                    quantity: 1,
                };
                let edited = vec![
                    self.edit(&fee_line(*plan), at.date_naive(), fixed, *months)
                        .await?,
                ];
                self.h
                    .services()
                    .schedule_amendment(
                        Actor::System,
                        self.sub(),
                        tenant,
                        SubscriptionAmendment {
                            apply_mode: PlanChangeMode::EndOfPeriod,
                            component_changes: ComponentChanges {
                                edited,
                                added: vec![],
                                removed: vec![],
                            },
                            add_on_changes: AddOnChanges::default(),
                        },
                    )
                    .await
                    .map_err(err)?;
            }
            Action::ChangeNow { at, to } => {
                let day = at.date_naive();
                let preview = self
                    .h
                    .services()
                    .preview_plan_change(
                        self.sub(),
                        tenant,
                        self.plans[*to],
                        self.seats_of(*to, model),
                        Some(PlanChangeMode::Immediate),
                    )
                    .await
                    .map_err(err)?;
                let result = self
                    .h
                    .services()
                    .apply_plan_change_immediate_at(
                        Actor::System,
                        self.sub(),
                        tenant,
                        self.plans[*to],
                        self.seats_of(*to, model),
                        day,
                    )
                    .await
                    .map_err(err)?;
                // R-011: a change in a free trial bills on the opening invoice, as its preview says.
                let opening = model.trialing(day);
                let document = if opening {
                    ChangeDocument::OpeningInvoice
                } else {
                    ChangeDocument::Adjustment
                };
                let billed_on = if opening {
                    result.first_invoice_id
                } else {
                    result.adjustment_invoice_id
                };
                let charged = match billed_on {
                    Some(id) => self.h.invoice(id).await.subtotal,
                    None => 0,
                };
                let (previewed, described) = preview
                    .proration
                    .map_or((0, document), |p| (p.charges_total_cents, p.document));
                if charged != previewed || described != document {
                    self.pending.push((
                        "preview".to_owned(),
                        format!(
                            "plan change on {day}: the preview charged {previewed} on {described:?}, the {document:?} bills {charged}"
                        ),
                    ));
                }
            }
            Action::ChangeAtEnd { to, seats, .. } => {
                // Seats set explicitly, or left to carry over at the change.
                let params = match seats {
                    Some(n) => self.seats_at(*to, *n, model),
                    None => self.capacity_of(*to, model),
                };
                self.h
                    .services()
                    .schedule_plan_change(
                        Actor::System,
                        self.sub(),
                        tenant,
                        self.plans[*to],
                        params,
                    )
                    .await
                    .map_err(err)?;
            }
            Action::OneTime { at, price } => {
                self.one_time += 1;
                let name = format!("Setup {}", self.one_time);
                let amendment = SubscriptionAmendment {
                    apply_mode: PlanChangeMode::Immediate,
                    component_changes: ComponentChanges {
                        edited: vec![],
                        added: vec![AddExtraComponent {
                            name: name.clone(),
                            product: ProductRef::New {
                                name: format!("{name} Product"),
                                fee_type: FeeTypeEnum::OneTime,
                                fee_structure: FeeStructure::OneTime {},
                            },
                            price_entry: PriceEntry::New(PriceInput {
                                cadence: BillingPeriodEnum::Monthly,
                                currency: "USD".to_owned(),
                                pricing: Pricing::OneTime {
                                    unit_price: cents(*price),
                                    quantity: 1,
                                },
                                credit_unit: None,
                            }),
                        }],
                        removed: vec![],
                    },
                    add_on_changes: AddOnChanges::default(),
                };
                self.amend_now(amendment, at.date_naive()).await?;
            }
            Action::CancelNow { at } => {
                self.h
                    .services()
                    .cancel_subscription(
                        Actor::System,
                        self.sub(),
                        tenant,
                        None,
                        CancellationEffectiveAt::Date(at.date_naive()),
                    )
                    .await
                    .map_err(err)?;
            }
            Action::CancelImmediately { .. } => {
                self.h
                    .services()
                    .cancel_subscription(
                        Actor::System,
                        self.sub(),
                        tenant,
                        None,
                        CancellationEffectiveAt::Immediate,
                    )
                    .await
                    .map_err(err)?;
            }
            Action::CancelBackdated { day, .. } => {
                self.h
                    .services()
                    .cancel_subscription(
                        Actor::System,
                        self.sub(),
                        tenant,
                        None,
                        CancellationEffectiveAt::Date(*day),
                    )
                    .await
                    .map_err(err)?;
            }
            Action::BuySeats { at, seats } | Action::RemoveSeats { at, seats } => {
                let delta = if matches!(action, Action::BuySeats { .. }) {
                    *seats
                } else {
                    -*seats
                };
                self.h
                    .services()
                    .update_subscription_slots_for_test(
                        tenant,
                        self.sub(),
                        self.seats[model.sub().plan],
                        i32::try_from(delta).map_err(err)?,
                        SlotUpgradeBillingMode::Optimistic,
                        Some(at.naive_utc()),
                    )
                    .await
                    .map_err(err)?;
            }
            Action::AttachAddOn { at, quantity } => {
                let add_on_changes = AddOnChanges {
                    added: vec![CreateSubscriptionAddOn {
                        add_on_id: self.add_on.expect("the add-on"),
                        customization: SubscriptionAddOnCustomization::None,
                        quantity: i32::try_from(*quantity).map_err(err)?,
                    }],
                    edited: vec![],
                    removed: vec![],
                };
                self.amend_add_on(add_on_changes, at.date_naive()).await?;
            }
            Action::AddOnQuantity { at, quantity } => {
                let day = at.date_naive();
                let live = self.live_add_on(day).await?;
                let add_on_changes = AddOnChanges {
                    added: vec![],
                    edited: vec![EditSubscriptionAddOn {
                        subscription_add_on_id: live,
                        quantity: Some(u32::try_from(*quantity).map_err(err)?),
                        customization: None,
                    }],
                    removed: vec![],
                };
                self.amend_add_on(add_on_changes, day).await?;
            }
            Action::DetachAddOn { at } => {
                let day = at.date_naive();
                let live = self.live_add_on(day).await?;
                let add_on_changes = AddOnChanges {
                    added: vec![],
                    edited: vec![],
                    removed: vec![live],
                };
                self.amend_add_on(add_on_changes, day).await?;
            }
            Action::CancelAtEnd { end, at, .. } => {
                self.h
                    .services()
                    .cancel_subscription_at(
                        Actor::System,
                        self.sub(),
                        tenant,
                        None,
                        CancellationEffectiveAt::Date(*end),
                        *at,
                    )
                    .await
                    .map_err(err)?;
            }
            Action::Void { at, pick } => {
                // A paid invoice is never voided: only an unpaid one can be.
                let unpaid: Vec<Invoice> = self
                    .with_status(InvoiceStatusEnum::Finalized)
                    .await
                    .into_iter()
                    .filter(|invoice| {
                        !matches!(
                            invoice.payment_status,
                            InvoicePaymentStatus::Paid | InvoicePaymentStatus::PartiallyPaid
                        )
                    })
                    .collect();
                if let Some(invoice) = pick_of(&unpaid, *pick) {
                    // An invoice its notes took back whole has nothing left to void: refused.
                    let voided = self
                        .h
                        .store()
                        .void_invoice_at(Actor::System, invoice.id, tenant, *at)
                        .await
                        .map_err(err);
                    match voided {
                        Err(e) if e.contains("No line items to credit") => {}
                        other => {
                            other?;
                        }
                    }
                }
            }
            Action::Pay { at, pick } => {
                let owing: Vec<Invoice> = self
                    .with_status(InvoiceStatusEnum::Finalized)
                    .await
                    .into_iter()
                    .filter(|invoice| invoice.amount_due > 0)
                    .collect();
                if let Some(invoice) = pick_of(&owing, *pick) {
                    self.h
                        .services()
                        .settle_invoice_payment_at(
                            Actor::System,
                            tenant,
                            invoice.id,
                            *at,
                            Some(format!("model-{}", invoice.id)),
                            InvoiceDelivery::Suppressed,
                        )
                        .await
                        .map_err(err)?;
                }
            }
            Action::FinalizeAll { at } => {
                for invoice in self.with_status(InvoiceStatusEnum::Draft).await {
                    self.issuing
                        .insert(invoice.id, common_clock::now().naive_utc());
                    self.h
                        .services()
                        .finalize_invoice_at(
                            Actor::System,
                            invoice.id,
                            tenant,
                            IssueAt::At(*at),
                            InvoiceDelivery::Suppressed,
                        )
                        .await
                        .map_err(err)?;
                }
            }
            Action::Finalize { at, pick } => {
                let drafts = self.with_status(InvoiceStatusEnum::Draft).await;
                if let Some(invoice) = pick_of(&drafts, *pick) {
                    self.issuing
                        .insert(invoice.id, common_clock::now().naive_utc());
                    self.h
                        .services()
                        .finalize_invoice_at(
                            Actor::System,
                            invoice.id,
                            tenant,
                            IssueAt::At(*at),
                            InvoiceDelivery::Suppressed,
                        )
                        .await
                        .map_err(err)?;
                }
            }
        }
        Ok(())
    }

    /// The seats the subscription carries onto plan `to`, and its capacity's commitment there.
    fn seats_of(&self, to: usize, model: &Model) -> Vec<ComponentParameterization> {
        self.seats_at(to, model.sub().seat_count, model)
    }

    fn seats_at(&self, to: usize, seats: i64, model: &Model) -> Vec<ComponentParameterization> {
        let mut params = vec![ComponentParameterization {
            component_id: self.seats[to],
            parameters: ComponentParameters {
                initial_slot_count: u32::try_from(seats).ok(),
                billing_period: None,
                committed_capacity: None,
            },
        }];
        params.extend(self.capacity_of(to, model));
        params
    }

    fn capacity_of(&self, to: usize, model: &Model) -> Vec<ComponentParameterization> {
        match (to, self.capacity, model.catalog().capacity) {
            (0, Some(component), Some(spec)) => vec![ComponentParameterization {
                component_id: component,
                parameters: ComponentParameters {
                    initial_slot_count: None,
                    billing_period: None,
                    committed_capacity: u64::try_from(spec.included).ok(),
                },
            }],
            _ => vec![],
        }
    }

    /// The add-on attached on `day`.
    async fn live_add_on(
        &self,
        day: NaiveDate,
    ) -> Result<common_domain::ids::SubscriptionAddOnId, String> {
        let add_on = self.add_on.expect("the add-on");
        self.h
            .store()
            .get_subscription_details(self.h.tenant, self.sub())
            .await
            .map_err(err)?
            .add_ons
            .iter()
            .filter(|a| {
                a.add_on_id == add_on
                    && a.effective_from <= day
                    && a.effective_to.is_none_or(|to| to > day)
            })
            .max_by_key(|a| a.effective_from)
            .map(|a| a.id)
            .ok_or_else(|| format!("no live add-on on {day}"))
    }

    async fn amend_add_on(&mut self, changes: AddOnChanges, day: NaiveDate) -> Result<(), String> {
        let amendment = SubscriptionAmendment {
            apply_mode: PlanChangeMode::Immediate,
            component_changes: ComponentChanges::default(),
            add_on_changes: changes,
        };
        self.amend_now(amendment, day).await
    }

    /// An edit of the live component named `name` to `pricing` every `months`, from `day`.
    async fn edit(
        &self,
        name: &str,
        day: NaiveDate,
        pricing: Pricing,
        months: u32,
    ) -> Result<EditComponent, String> {
        let details = self
            .h
            .store()
            .get_subscription_details(self.h.tenant, self.sub())
            .await
            .map_err(err)?;
        let component = details
            .price_components
            .iter()
            .filter(|c| {
                c.name == name
                    && c.effective_from <= day
                    && c.effective_to.is_none_or(|to| to > day)
            })
            .max_by_key(|c| c.effective_from)
            .ok_or_else(|| format!("no live component {name} on {day}"))?;
        Ok(EditComponent {
            subscription_component_id: component.id,
            name: None,
            price_entry: PriceEntry::New(PriceInput {
                cadence: cadence(months),
                currency: "USD".to_owned(),
                pricing,
                credit_unit: None,
            }),
        })
    }

    /// Previews `amendment`, applies it on `day`, and records where what it issued differs from
    /// what the preview said: the adjustment invoice's figures, its amount due included, and the
    /// notes' total.
    async fn amend_now(
        &mut self,
        amendment: SubscriptionAmendment,
        day: NaiveDate,
    ) -> Result<(), String> {
        let tenant = self.h.tenant;
        let preview = self
            .h
            .services()
            .preview_amendment(self.sub(), tenant, amendment.clone())
            .await
            .map_err(err)?;
        let result = self
            .h
            .services()
            .apply_amendment_immediate_at(Actor::System, self.sub(), tenant, amendment, day)
            .await
            .map_err(err)?;
        let issued = match result.adjustment_invoice_id {
            Some(id) => {
                let invoice = self.h.invoice(id).await;
                Some((invoice.subtotal, invoice.total, invoice.amount_due))
            }
            None => None,
        };
        let previewed = preview
            .adjustment_invoice
            .as_ref()
            .map(|content| (content.subtotal, content.total, content.amount_due))
            .filter(|figures| *figures != (0, 0, 0));
        if issued.filter(|figures| *figures != (0, 0, 0)) != previewed {
            self.pending.push((
                "preview".to_owned(),
                format!(
                    "amendment on {day}: the preview's adjustment (subtotal, total, due) {previewed:?}, issued {issued:?}"
                ),
            ));
        }
        let notes: i64 = if result.credit_note_ids.is_empty() {
            0
        } else {
            self.h
                .store()
                .list_credit_notes_by_ids(result.credit_note_ids.clone())
                .await
                .map_err(err)?
                .iter()
                .map(|note| note.total)
                .sum()
        };
        let previewed_notes = preview
            .credit_note
            .as_ref()
            .map_or(0, |content| content.total);
        if notes.abs() != previewed_notes.abs() {
            self.pending.push((
                "preview".to_owned(),
                format!(
                    "amendment on {day}: the preview's credit note totals {previewed_notes}, the notes issued {notes}"
                ),
            ));
        }
        Ok(())
    }

    async fn start(&mut self, day: NaiveDate, catalog: &Catalog) -> Result<(), String> {
        let h = &self.h;
        let tenant = h.tenant;
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
                    name: "Lifecycle units".to_owned(),
                    description: None,
                    code: "lifecycle_units".to_owned(),
                    aggregation_type: BillingMetricAggregateEnum::Sum,
                    aggregation_key: Some(PROPERTY.to_owned()),
                    unit_conversion_factor: None,
                    unit_conversion_rounding: None,
                    segmentation_matrix: None,
                    filters: None,
                    usage_group_key: None,
                    usage_breakdown_key: None,
                    unit_label: None,
                    tenant_id: tenant,
                    product_family_id: family,
                    product_id: None,
                },
            )
            .await
            .map_err(err)?;
        // Both plans sell their seats under one product, so a change carries them (R-108).
        let mut seat_product: Option<common_domain::ids::ProductId> = None;
        for (p, spec) in catalog.plans.iter().enumerate() {
            let PlanSpec {
                fee,
                arrears,
                months,
                rate,
                seat,
            } = *spec;
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
                                currency: "USD".to_owned(),
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
                            name: format!("Lifecycle plan {}", plan_name(p)),
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
                            currency: Some("USD".to_owned()),
                            billing_cycles: None,
                            // A free trial is the plan version's: a trial the request alone
                            // asks for is a paid one.
                            trial: catalog.trial.filter(|_| p == 0).map(|trial| PlanTrial {
                                duration_days: u32::try_from((trial.end - day).num_days())
                                    .unwrap_or(0),
                                trialing_plan_id: None,
                                trial_is_free: true,
                            }),
                            entitlements: vec![],
                            tax_inclusive: false,
                            credit_exhausted_rule: None,
                        },
                        price_components: vec![
                            component(
                                fee_line(p),
                                FeeType::ExtraRecurring {
                                    unit_price: cents(fee),
                                    quantity: 1,
                                    billing_type: if arrears {
                                        BillingType::Arrears
                                    } else {
                                        BillingType::Advance
                                    },
                                    cadence: cadence(months),
                                },
                            ),
                            component(
                                units_line(p),
                                FeeType::Usage {
                                    metric_id: metric.id,
                                    pricing: UsagePricingModel::PerUnit { rate: cents(rate) },
                                    cadence: cadence(months),
                                },
                            ),
                        ]
                        .into_iter()
                        .chain(catalog.capacity.filter(|_| p == 0).map(|capacity| {
                            component(
                                "A capacity".to_owned(),
                                FeeType::Capacity {
                                    metric_id: metric.id,
                                    thresholds: vec![CapacityThreshold {
                                        included_amount: u64::try_from(capacity.included)
                                            .unwrap_or(0),
                                        price: cents(capacity.fee),
                                        per_unit_overage: cents(capacity.overage),
                                    }],
                                    cadence: cadence(months),
                                },
                            )
                        }))
                        .chain([{
                            let mut seats = component(
                                seats_line(p),
                                FeeType::Slot {
                                    rates: vec![TermRate {
                                        term: cadence(months),
                                        price: cents(seat),
                                    }],
                                    slot_unit_name: "seat".to_owned(),
                                    upgrade_policy: UpgradePolicy::Prorated,
                                    downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
                                    minimum_count: Some(1),
                                    quota: None,
                                },
                            );
                            if let Some(product) = seat_product {
                                seats.product_ref = ProductRef::Existing(product);
                            }
                            seats
                        }])
                        .collect(),
                    },
                )
                .await
                .map_err(err)?;
            self.plans.push(created.version.id);
            let seats = created
                .price_components
                .iter()
                .find(|c| c.name == seats_line(p))
                .ok_or("no seats component")?;
            seat_product = seat_product.or(seats.product_id);
            self.seats.push(seats.id);
            if p == 0 {
                self.capacity = created
                    .price_components
                    .iter()
                    .find(|c| c.name == "A capacity")
                    .map(|c| c.id);
            }
        }
        let spec = catalog.add_on;
        let add_on = h
            .store()
            .create_add_on_from_ref(
                "Lifecycle add-on".to_owned(),
                ProductRef::New {
                    name: "Lifecycle add-on".to_owned(),
                    fee_type: FeeTypeEnum::Rate,
                    fee_structure: FeeStructure::Rate {},
                },
                PriceEntry::New(PriceInput {
                    cadence: cadence(spec.months),
                    currency: "USD".to_owned(),
                    pricing: Pricing::Rate {
                        rate: cents(spec.price),
                    },
                    credit_unit: None,
                }),
                None,
                true,
                Some(10),
                tenant,
                family,
                vec![],
            )
            .await
            .map_err(err)?;
        for (p, plan) in self.plans.iter().enumerate() {
            if catalog.offers_add_on(p) {
                h.store()
                    .attach_add_on_to_plan_version(PlanVersionAddOnNew {
                        plan_version_id: *plan,
                        add_on_id: add_on.id,
                        price_id: None,
                        self_serviceable: None,
                        max_instances_per_subscription: None,
                        tenant_id: tenant,
                    })
                    .await
                    .map_err(err)?;
            }
        }
        self.add_on = Some(add_on.id);
        let customer = h.customer("Lifecycle", Tax::Untaxed).await;
        self.customer = Some(customer);
        let mut request = h.request(customer, self.plans[0], day);
        request.subscription.auto_advance_invoices = catalog.auto_advance;
        request.subscription.end_date = catalog.term_end;
        if let Some(trial) = catalog.trial {
            request.subscription.trial_duration =
                Some(u32::try_from((trial.end - day).num_days()).map_err(err)?);
            request.subscription.billing_day_anchor =
                trial.anchor.map(|d| u16::try_from(d).unwrap_or(1));
            if !trial.card {
                request.subscription.activation_condition =
                    SubscriptionActivationCondition::OnCheckout;
            }
        }
        if let Some(minimum) = catalog.minimum {
            request.minimum_override = Some((minimum, CreateMinimumScope::AllComponents));
        }
        if let Some(coupon) = catalog.coupon {
            let discount = match coupon.off {
                Off::Percent(p) => CouponDiscount::Percentage(Decimal::from(p)),
                Off::Fixed(amount) => CouponDiscount::Fixed {
                    amount: cents(amount),
                    currency: "USD".to_owned(),
                },
            };
            let created = h
                .services()
                .create_coupon_at(
                    Actor::System,
                    CouponNew {
                        code: "LIFECYCLE".to_owned(),
                        description: "Lifecycle coupon".to_owned(),
                        tenant_id: tenant,
                        discount,
                        expires_at: coupon.expires.map(|d| midnight(d).naive_utc()),
                        redemption_limit: None,
                        recurring_value: coupon.invoices.map(|n| n as i32),
                        reusable: false,
                        plan_ids: vec![],
                    },
                    midnight(day) - Duration::days(1),
                )
                .await
                .map_err(err)?;
            request.coupons = Some(CreateSubscriptionCoupons {
                coupons: vec![CreateSubscriptionCoupon {
                    coupon_id: created.id,
                }],
            });
        }
        let mut parameterized = vec![ComponentParameterization {
            component_id: self.seats[0],
            parameters: ComponentParameters {
                initial_slot_count: u32::try_from(catalog.seats).ok(),
                billing_period: None,
                committed_capacity: None,
            },
        }];
        if let (Some(component), Some(spec)) = (self.capacity, catalog.capacity) {
            parameterized.push(ComponentParameterization {
                component_id: component,
                parameters: ComponentParameters {
                    initial_slot_count: None,
                    billing_period: None,
                    committed_capacity: u64::try_from(spec.included).ok(),
                },
            });
        }
        request.price_components = Some(CreateSubscriptionComponents {
            parameterized_components: parameterized,
            overridden_components: vec![],
            extra_components: vec![],
            remove_components: vec![],
        });
        // A trial starts now, the clock on its start: only a paid start is backdated.
        let created = if catalog.trial.is_some() {
            h.services()
                .insert_subscription(Actor::System, request, tenant)
                .await
                .map_err(err)?
                .id
        } else {
            h.services()
                .insert_subscription_started_at(
                    Actor::System,
                    request,
                    tenant,
                    midnight(day),
                    InvoiceDelivery::Suppressed,
                )
                .await
                .map_err(err)?
                .0
                .id
        };
        self.sub = Some(created);
        if let Some(fee) = catalog.companion {
            self.start_companion(day, catalog, family, fee).await?;
        }
        Ok(())
    }

    /// A second subscription of the customer from `day`: one monthly fee in advance, its renewals
    /// consolidated with the subscription's.
    async fn start_companion(
        &mut self,
        day: NaiveDate,
        catalog: &Catalog,
        family: common_domain::ids::ProductFamilyId,
        fee: i64,
    ) -> Result<(), String> {
        let h = &self.h;
        let tenant = h.tenant;
        let fee_type = FeeType::ExtraRecurring {
            unit_price: cents(fee),
            quantity: 1,
            billing_type: BillingType::Advance,
            cadence: BillingPeriodEnum::Monthly,
        };
        let (fee_kind, fee_structure) = extract_fee_structure(&fee_type);
        let plan = h
            .store()
            .insert_plan(
                Actor::System,
                FullPlanNew {
                    plan: PlanNew {
                        name: "Lifecycle companion".to_owned(),
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
                        currency: Some("USD".to_owned()),
                        billing_cycles: None,
                        trial: None,
                        entitlements: vec![],
                        tax_inclusive: false,
                        credit_exhausted_rule: None,
                    },
                    price_components: vec![PriceComponentNewInternal {
                        name: "Companion fee".to_owned(),
                        product_ref: ProductRef::New {
                            name: "Companion fee".to_owned(),
                            fee_type: fee_kind,
                            fee_structure,
                        },
                        prices: extract_pricing(&fee_type)
                            .into_iter()
                            .map(|(cadence, pricing)| {
                                PriceEntry::New(PriceInput {
                                    cadence,
                                    currency: "USD".to_owned(),
                                    pricing,
                                    credit_unit: None,
                                })
                            })
                            .collect(),
                    }],
                },
            )
            .await
            .map_err(err)?;
        let mut request = h.request(self.customer.expect("started"), plan.version.id, day);
        request.subscription.auto_advance_invoices = catalog.auto_advance;
        request.subscription.end_date = catalog.term_end;
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
        self.companion = Some(created.id);
        // Both started, the period renewals consolidate from here.
        let entity = h
            .store()
            .get_invoicing_entity(tenant, None)
            .await
            .map_err(err)?;
        h.store()
            .patch_invoicing_entity(
                Actor::System,
                InvoicingEntityPatch {
                    id: entity.id,
                    consolidate_recurring_invoices: Some(true),
                    ..Default::default()
                },
                tenant,
            )
            .await
            .map_err(err)?;
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
            // A consolidated child is part of its parent, the document issued.
            .filter(|i| i.consolidated_into_invoice_id.is_none())
            .collect();
        invoices.sort_by_key(|i| (i.created_at, i.id.as_uuid()));
        invoices
    }

    async fn with_status(&self, status: InvoiceStatusEnum) -> Vec<Invoice> {
        self.all_invoices()
            .await
            .into_iter()
            .filter(|invoice| invoice.status == status)
            .collect()
    }

    async fn notes(&self) -> Vec<CreditNote> {
        let Some(customer) = self.customer else {
            return vec![];
        };
        self.h
            .store()
            .list_credit_notes_by_customer_id(self.h.tenant, customer)
            .await
            .expect("credit notes")
    }

    /// What the documents bill net of their notes, and what voided invoices billed net of theirs,
    /// the note a void writes for its own invoice left out (R-096).
    async fn billed(&self) -> (i64, i64) {
        let invoices = self.all_invoices().await;
        let voided: Vec<InvoiceId> = invoices
            .iter()
            .filter(|invoice| invoice.status == InvoiceStatusEnum::Void)
            .map(|invoice| invoice.id)
            .collect();
        let notes = self.notes().await;
        let note_sum = |of_void: bool| -> i64 {
            notes
                .iter()
                .filter(|note| note.status != CreditNoteStatus::Voided)
                .filter(|note| note.origin != CreditNoteOrigin::Void)
                .filter(|note| voided.contains(&note.invoice_id) == of_void)
                .map(|note| {
                    note.subtotal
                        - note
                            .line_items
                            .iter()
                            .filter(|line| line.name == MINIMUM_LINE)
                            .map(|line| line.amount_subtotal)
                            .sum::<i64>()
                })
                .sum()
        };
        // The minimum's true-ups are checked on their own (`minimum_differences`).
        let live: i64 = invoices
            .iter()
            .filter(|i| issued(i))
            .map(|i| i.subtotal - Self::shortfall(i))
            .sum();
        let void: i64 = invoices
            .iter()
            .filter(|i| i.status == InvoiceStatusEnum::Void)
            .map(|i| i.subtotal)
            .sum();
        (live + note_sum(false), void + note_sum(true))
    }

    /// The figures the model fixes: the period the engine is in while nothing is pending, and,
    /// once the subscription ended, what its documents billed over its life against what the
    /// reference owes: exactly, or within what voids forgave (R-079, R-080, R-096).
    pub async fn compare(&self, model: &Model) -> Vec<Diff> {
        let mut diffs = self.pending.clone();
        let Some(sub) = self.sub else {
            return diffs;
        };
        let expected = model.sub();
        // A trial without a card awaits its checkout, no period stored.
        let waiting = model
            .catalog()
            .trial
            .is_some_and(|trial| !trial.card && model.trialing(expected.period_start));
        if expected.active && expected.scheduled.is_none() && !waiting {
            let details = self
                .h
                .store()
                .get_subscription_details(self.h.tenant, sub)
                .await
                .expect("the subscription");
            let end = details.subscription.current_period_end;
            if end != Some(expected.period_end) {
                diffs.push((
                    "period".to_owned(),
                    format!(
                        "R-084: the period ends {end:?}, the model's on {}",
                        expected.period_end
                    ),
                ));
            }
        }
        // A draft is no issued document: its finalize settles what it bills (R-080).
        let drafts = self.with_status(InvoiceStatusEnum::Draft).await;
        if let Some(owed) = model.owed().filter(|_| drafts.is_empty()) {
            let (live, voided) = self.billed().await;
            let wrong = if voided == 0 {
                live != owed
            } else {
                live > owed || live + voided < owed
            };
            if wrong {
                diffs.push((
                    "lifetime".to_owned(),
                    format!(
                        "the documents bill {live} net of their notes ({voided} voided), the reference owes {owed}"
                    ),
                ));
            }
            diffs.extend(self.coupon_differences(model).await);
            diffs.extend(self.minimum_differences(model).await);
        }
        diffs
    }

    fn shortfall(invoice: &Invoice) -> i64 {
        invoice
            .line_items
            .iter()
            .filter(|line| line.name == MINIMUM_LINE)
            .map(|line| line.amount_subtotal)
            .sum()
    }

    /// R-020, R-089: the coupon takes its share off each period invoice, never off a change's
    /// documents nor the minimum's shortfall (R-022); its uses go to the oldest periods first,
    /// whatever order the drafts are issued in, and a void gives its use back (R-096).
    async fn coupon_differences(&self, model: &Model) -> Vec<Diff> {
        let Some(coupon) = model.catalog().coupon else {
            return vec![];
        };
        let invoices: Vec<Invoice> = self
            .all_invoices()
            .await
            .into_iter()
            .filter(|i| i.subscription_id == self.sub && i.status != InvoiceStatusEnum::Closed)
            .collect();
        // Each finalize, then each void, in the order they happened.
        let mut events: Vec<(chrono::NaiveDateTime, bool, &Invoice)> = invoices
            .iter()
            .filter_map(|i| i.finalized_at.map(|at| (at, false, i)))
            .chain(
                invoices
                    .iter()
                    .filter_map(|i| i.voided_at.map(|at| (at, true, i))),
            )
            .collect();
        events.sort_by_key(|(at, void, i)| (*at, *void, i.created_at, i.id.as_uuid()));
        let mut used: u32 = 0;
        // R-089: an invoice the coupon lasts is one of its invoices, whatever it took off (a
        // percentage of a few cents truncates to none, KW-01).
        let mut counted: Vec<InvoiceId> = vec![];
        let mut found = vec![];
        let period = |invoice: &Invoice| {
            matches!(
                invoice.invoice_type,
                InvoiceType::Recurring | InvoiceType::Final
            ) && invoice.subtotal - Self::shortfall(invoice) > 0
        };
        let place = |invoice: &Invoice| {
            (
                invoice.billing_period_start,
                invoice.created_at,
                invoice.id.as_uuid(),
            )
        };
        for (at, void, invoice) in events {
            if void {
                if counted.contains(&invoice.id) {
                    used = used.saturating_sub(1);
                }
                continue;
            }
            let base = invoice.subtotal - Self::shortfall(invoice);
            let off = match coupon.off {
                // KW-01: a percentage truncates to the minor unit, where R-001 would round.
                Off::Percent(p) => base * p / 100,
                Off::Fixed(amount) => amount.min(base),
            };
            let ahead = invoices
                .iter()
                .filter(|other| {
                    period(other)
                        && place(other) < place(invoice)
                        && other.created_at <= at
                        && other.finalized_at.is_none_or(|issued| issued > at)
                })
                .count();
            let ahead = u32::try_from(ahead).unwrap_or(u32::MAX);
            let applies = period(invoice)
                && coupon
                    .invoices
                    .is_none_or(|n| used.saturating_add(ahead) < n);
            let expected = if applies { off } else { 0 };
            if applies {
                used += 1;
                counted.push(invoice.id);
            }
            if invoice.discount != expected {
                found.push(format!(
                    "{:?} of {} (subtotal {}, shortfall {}): discount {}, expected {expected}",
                    invoice.invoice_type,
                    invoice.invoice_date,
                    invoice.subtotal,
                    Self::shortfall(invoice),
                    invoice.discount
                ));
            }
        }
        let live = invoices
            .iter()
            .filter(|i| i.status != InvoiceStatusEnum::Void && i.discount != 0)
            .count();
        if let Some(n) = coupon
            .invoices
            .filter(|n| live > usize::try_from(*n).unwrap_or(0))
        {
            found.push(format!("{live} invoices discounted, the coupon lasts {n}"));
        }
        if found.is_empty() {
            vec![]
        } else {
            vec![("coupon".to_owned(), found.join("; "))]
        }
    }

    /// The period invoices holding or owed one of a limited coupon's uses at `at` (R-089): those
    /// issued by then that took one, then the open drafts, oldest period first, while one is left.
    fn coupon_uses_at(
        &self,
        coupon: &CouponSpec,
        invoices: &[Invoice],
        at: chrono::NaiveDateTime,
    ) -> Option<Vec<InvoiceId>> {
        let n = usize::try_from(coupon.invoices?).unwrap_or(usize::MAX);
        let period = |invoice: &&Invoice| {
            matches!(
                invoice.invoice_type,
                InvoiceType::Recurring | InvoiceType::Final
            ) && invoice.subtotal - Self::shortfall(invoice) > 0
                && invoice.subscription_id == self.sub
                && invoice.status != InvoiceStatusEnum::Closed
                && invoice.voided_at.is_none_or(|voided| voided > at)
        };
        let mut owed: Vec<InvoiceId> = invoices
            .iter()
            .filter(period)
            .filter(|i| i.discount != 0 && i.finalized_at.is_some_and(|issued| issued <= at))
            .map(|i| i.id)
            .collect();
        let mut open: Vec<&Invoice> = invoices
            .iter()
            .filter(period)
            .filter(|i| i.created_at <= at && i.finalized_at.is_none_or(|issued| issued > at))
            .collect();
        open.sort_by_key(|i| (i.billing_period_start, i.created_at, i.id.as_uuid()));
        let left = n.saturating_sub(owed.len());
        owed.extend(open.into_iter().take(left).map(|i| i.id));
        Some(owed)
    }

    /// R-022 as decided: a period pays at least its minimum, prorated by its days. Its spend is
    /// the lines of its days net of coupons and of the engine's notes, gross of credit; the
    /// shortfall line, never discounted, brings it to the minimum. A period a backdated ending
    /// cut keeps the true-up it closed with, and pays more only past it, by its committed days.
    async fn minimum_differences(&self, model: &Model) -> Vec<Diff> {
        let Some(minimum) = model.catalog().minimum else {
            return vec![];
        };
        let invoices: Vec<Invoice> = self
            .all_invoices()
            .await
            .into_iter()
            .filter(issued)
            .collect();
        let notes: Vec<CreditNote> = self
            .notes()
            .await
            .into_iter()
            .filter(|note| {
                note.status != CreditNoteStatus::Voided && note.origin != CreditNoteOrigin::Void
            })
            .collect();
        let all = self.all_invoices().await;
        // R-096 forgives a period's true-up when the invoice carrying it was voided, or when a
        // document of the period was voided after it was computed; any other void is no spend.
        let forgiven = |from: NaiveDate, to: NaiveDate| {
            let in_period = |i: &Invoice| {
                i.line_items
                    .iter()
                    .any(|line| from <= line.start_date && line.start_date < to)
            };
            // A backdated ending's final invoice, dated the day it is issued, trues up the period
            // its ending cut (R-016, R-022). Issuing a draft dates it that day: a recurring
            // invoice carries the period it opens after, never the one its issue day ends.
            let ended_on_to = model.sub().ended == Some(to);
            let carries = |i: &Invoice| {
                (matches!(i.invoice_type, InvoiceType::Recurring | InvoiceType::Final)
                    && i.billing_period_start == to)
                    || (i.invoice_type == InvoiceType::Final
                        && (i.invoice_date == to || ended_on_to))
                    || i.line_items.iter().any(|line| {
                        line.name == MINIMUM_LINE && from <= line.start_date && line.start_date < to
                    })
            };
            // A closed invoice, empty, was never finalized: it was trued up when written.
            let trued_up = all
                .iter()
                .filter(|i| carries(i))
                .map(|i| i.finalized_at.or(i.updated_at).unwrap_or(i.created_at))
                .max();
            all.iter()
                .filter(|i| i.status == InvoiceStatusEnum::Void && in_period(i))
                .any(|i| {
                    carries(i)
                        || trued_up.is_some_and(|at| i.voided_at.is_none_or(|voided| voided > at))
                })
        };
        let mut found = vec![];
        let floor_of = |from: NaiveDate, to: NaiveDate, full: i64| {
            rounded(
                i128::from(minimum) * i128::from((to - from).num_days().max(0)),
                i128::from(full),
            )
        };
        for (from, to, full) in &model.sub().periods {
            if forgiven(*from, *to) {
                continue;
            }
            let cut = model.sub().cut.filter(|cut| cut < to);
            let floor = floor_of(*from, *to, *full);
            // Its close counted the notes issued by then. A renewal is stamped finalized at its
            // period's instant, though written after that day's notes; a draft issued by hand
            // later, at the moment asked, though written after the notes issued before it.
            let closed_at = all
                .iter()
                .filter(|i| {
                    i.status != InvoiceStatusEnum::Void
                        && i.line_items.iter().any(|line| {
                            line.name == MINIMUM_LINE
                                && line.start_date == *from
                                && line.end_date == *to
                        })
                })
                .filter_map(|i| {
                    let written = self.issuing.get(&i.id).copied().unwrap_or(i.created_at);
                    i.finalized_at.map(|at| at.max(written))
                })
                .max();
            let ended_by = model.sub().ended.is_some_and(|ended| ended <= *to);
            // What the ending gives back of the period counts whenever it is issued: its own
            // document trues the period up again (R-022).
            let counts = |note: &CreditNote| {
                ended_by
                    || note.origin == CreditNoteOrigin::Cancellation
                    || closed_at.is_none_or(|at| note.finalized_at.unwrap_or(note.created_at) <= at)
            };
            // A use a void gave a draft of the period after its close leaves its true-up as
            // that close counted it (R-022, R-089).
            let owed_at_close = closed_at.and_then(|at| {
                let coupon = model.catalog().coupon?;
                Some((at, self.coupon_uses_at(&coupon, &all, at)?))
            });
            let late_use = |invoice: &Invoice| {
                owed_at_close.as_ref().is_some_and(|(at, owed)| {
                    invoice.discount != 0
                        && invoice.created_at <= *at
                        && invoice.finalized_at.is_some_and(|issued| issued > *at)
                        && !owed.contains(&invoice.id)
                })
            };
            let mut spend = 0;
            let mut gross = 0;
            let mut paid = 0;
            let mut closed_with = 0;
            for invoice in &invoices {
                let late = cut.is_none() && late_use(invoice);
                for line in &invoice.line_items {
                    if line.start_date < *from || line.start_date >= *to {
                        continue;
                    }
                    let credited: i64 = notes
                        .iter()
                        .filter(|note| note.invoice_id == invoice.id && counts(note))
                        .flat_map(|note| &note.line_items)
                        .filter(|c| c.local_id == line.local_id)
                        .map(|c| c.amount_total.abs())
                        .sum();
                    // A shortfall is never credited back (R-022): paid net of any note on it.
                    if line.name == MINIMUM_LINE {
                        paid += line.amount_total - credited;
                        if line.end_date == *to {
                            closed_with += line.amount_total - credited;
                        }
                    } else {
                        let discount = if late {
                            line.amount_subtotal - line.taxable_amount
                        } else {
                            0
                        };
                        spend += line.amount_total - credited + discount;
                        gross += line.amount_total;
                    }
                }
            }
            if let Some(cut) = cut {
                // Its close trued up the whole floor against a spend no later note had reduced,
                // unless the ending closed that renewal, a draft, unissued (R-080). A document
                // voided after the close was spend when it counted (R-096).
                let renewed = all
                    .iter()
                    .filter(|i| {
                        i.invoice_type == InvoiceType::Recurring && i.billing_period_start == *to
                    })
                    .filter_map(|i| {
                        let written = self.issuing.get(&i.id).copied().unwrap_or(i.created_at);
                        i.finalized_at.map(|at| at.max(written))
                    })
                    .min();
                let voided_later: i64 = all
                    .iter()
                    .filter(|i| {
                        renewed.is_some_and(|close| {
                            i.status == InvoiceStatusEnum::Void
                                && i.finalized_at.is_some_and(|issued| issued <= close)
                                && i.voided_at.is_some_and(|voided| voided > close)
                        })
                    })
                    .flat_map(|i| &i.line_items)
                    .filter(|line| {
                        line.name != MINIMUM_LINE
                            && *from <= line.start_date
                            && line.start_date < *to
                    })
                    .map(|line| line.amount_total)
                    .sum();
                let (least, most) = (
                    (floor - gross - voided_later).max(0),
                    (floor - spend - voided_later).max(0),
                );
                let unissued = all.iter().any(|i| {
                    i.status == InvoiceStatusEnum::Closed
                        && (i.billing_period_start == *to || i.invoice_date == *to)
                });
                let closed =
                    (least..=most).contains(&closed_with) || (closed_with == 0 && unissued);
                let prorated = floor_of(*from, cut.max(*from), *full);
                let owed = closed_with.max((prorated - spend).max(0));
                if !closed || paid != owed {
                    found.push(format!(
                        "{from}..{to} cut on {cut}: spend {spend} net of coupons and notes ({gross} gross), minimum {floor}, {prorated} by its committed days: trued up {closed_with} at its close (between {least} and {most}), shortfall paid {paid}, expected {owed}"
                    ));
                }
                continue;
            }
            let owed = (floor - spend).max(0);
            if paid != owed {
                found.push(format!(
                    "{from}..{to}: spend {spend} net of coupons and notes, minimum {floor}: shortfall paid {paid}, expected {owed}"
                ));
            }
        }
        if found.is_empty() {
            vec![]
        } else {
            vec![("minimum".to_owned(), found.join("; "))]
        }
    }

    pub async fn fingerprint(&self) -> Vec<String> {
        let mut seen: Vec<String> = self
            .all_invoices()
            .await
            .iter()
            .map(|i| {
                format!(
                    "{} {:?} {:?} {} {} {} {}",
                    i.id,
                    i.status,
                    i.payment_status,
                    i.subtotal,
                    i.total,
                    i.amount_due,
                    i.line_items.len()
                )
            })
            .collect();
        seen.extend(
            self.notes()
                .await
                .iter()
                .map(|n| format!("note {} {:?} {}", n.id, n.status, n.subtotal)),
        );
        seen
    }

    /// The scheduled events, the billing controller and the period's billing a second time:
    /// none may write.
    pub async fn rerun_workers(&self, action: &Action) -> Result<(), String> {
        // At the action's instant again: a draft refreshed an hour before by the previous action
        // turns stale within the milliseconds the first pass took, and is refreshed on its hour.
        clock(action.at());
        self.settle().await?;
        if let (Action::Advance { at }, Some(sub), None) = (action, self.sub, self.companion)
            && let Ok(Some(invoice)) = self
                .h
                .services()
                .bill_subscription_period_at(self.h.tenant, sub, *at, InvoiceDelivery::Suppressed)
                .await
        {
            return Err(format!(
                "billing the period again at {at} returned invoice {}",
                invoice.invoice.id
            ));
        }
        Ok(())
    }

    pub async fn dump(&self) -> Vec<String> {
        let mut lines = vec![];
        for invoice in self.all_invoices().await {
            lines.push(format!(
                "document: {:?} {:?} dated {} subtotal {} total {} due {}",
                invoice.invoice_type,
                invoice.status,
                invoice.invoice_date,
                invoice.subtotal,
                invoice.total,
                invoice.amount_due
            ));
            for l in &invoice.line_items {
                lines.push(format!(
                    "document:   {} {}..{} qty {:?} x {:?} = {}",
                    l.name, l.start_date, l.end_date, l.quantity, l.unit_price, l.amount_subtotal
                ));
            }
        }
        for note in self.notes().await {
            lines.push(format!(
                "note: on {} {:?} subtotal {}",
                note.invoice_number, note.status, note.subtotal
            ));
            for l in &note.line_items {
                lines.push(format!(
                    "note:   {} {}..{} = {}",
                    l.name, l.start_date, l.end_date, l.amount_subtotal
                ));
            }
        }
        lines
    }

    /// The credit invariants, and conservation: no note takes more from a line than it billed,
    /// and no fee period is billed whole on two issued documents (R-080).
    pub async fn invariants(&self) -> Vec<String> {
        let mut conn = self.h.env.conn().await;
        let mut found =
            crate::helpers::invariants::credit_invariant_violations(&mut conn, self.h.tenant).await;
        let invoices = self.all_invoices().await;
        let notes: Vec<CreditNote> = self
            .notes()
            .await
            .into_iter()
            .filter(|note| {
                note.status != CreditNoteStatus::Voided && note.origin != CreditNoteOrigin::Void
            })
            .collect();
        for invoice in &invoices {
            for line in invoice.line_items.iter().filter(|l| l.amount_subtotal > 0) {
                let taken: i64 = notes
                    .iter()
                    .filter(|note| note.invoice_id == invoice.id)
                    .flat_map(|note| &note.line_items)
                    .filter(|credited| credited.local_id == line.local_id)
                    .map(|credited| credited.amount_subtotal.abs())
                    .sum();
                if taken > line.amount_subtotal {
                    found.push(format!(
                        "notes take {taken} from {} {}..{} on {}, which billed {}",
                        line.name,
                        line.start_date,
                        line.end_date,
                        invoice.invoice_date,
                        line.amount_subtotal
                    ));
                }
            }
        }
        // A draft's line of a charge a change ended is taken back when the draft is issued
        // (R-080): only issued documents can bill a period twice.
        let live: Vec<&Invoice> = invoices
            .iter()
            .filter(|i| issued(i) && i.status != InvoiceStatusEnum::Draft)
            .collect();
        let mut seen: Vec<(String, NaiveDate, NaiveDate, i64, InvoiceId)> = vec![];
        for invoice in &live {
            for line in &invoice.line_items {
                // Seats bought twice in a period are two purchases of the same days, so two
                // identical "3 seat upgrade" lines are two purchases, not one billed twice.
                if line.amount_subtotal <= 0
                    || line.name.contains("seats")
                    || line.name.ends_with("seat upgrade")
                {
                    continue;
                }
                let credited = notes.iter().any(|note| {
                    note.invoice_id == invoice.id
                        && note.line_items.iter().any(|c| c.local_id == line.local_id)
                });
                if credited {
                    continue;
                }
                let key = (
                    line.name.clone(),
                    line.start_date,
                    line.end_date,
                    line.amount_subtotal,
                );
                if let Some(first) = seen.iter().find(|(n, a, b, m, id)| {
                    (n, *a, *b, *m) == (&key.0, key.1, key.2, key.3) && *id != invoice.id
                }) {
                    found.push(format!(
                        "{} {}..{} billed {} on two documents ({} and {})",
                        key.0, key.1, key.2, key.3, first.4, invoice.id
                    ));
                }
                seen.push((key.0, key.1, key.2, key.3, invoice.id));
            }
        }
        found
    }
}

fn pick_of(invoices: &[Invoice], pick: u8) -> Option<&Invoice> {
    (!invoices.is_empty()).then(|| &invoices[usize::from(pick) % invoices.len()])
}
