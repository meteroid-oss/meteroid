//! Usage attribution on resources (ADR 0011 §3.6): which of a customer's events each subscription
//! bills. A metric attributed to a type routes an event to the subscriptions of the resource whose
//! key value it carries, the older first, else to the customer's own subscriptions.
use crate::domain::BillableMetric;
use crate::domain::resources::ResourceType;
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, BillableMetricId, ResourceTypeId, SubscriptionId};
use serde::Serialize;
use std::collections::HashMap;

/// The events of one resource: the first of `properties` an event has holds `value`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ResourceMatch {
    pub properties: Vec<String>,
    pub value: String,
}

impl ResourceMatch {
    pub fn matches(&self, properties: &HashMap<String, String>) -> bool {
        self.properties
            .iter()
            .find_map(|p| properties.get(p).filter(|v| !v.is_empty()))
            == Some(&self.value)
    }
}

/// A subscription's resource as attribution reads it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttributedResource {
    pub type_id: ResourceTypeId,
    /// The type's keys, canonical first.
    pub keys: Vec<String>,
    pub key_value: String,
}

impl AttributedResource {
    pub fn of(resource_type: &ResourceType, key_value: &str) -> Self {
        Self {
            type_id: resource_type.id,
            keys: resource_type.keys.clone(),
            key_value: key_value.to_owned(),
        }
    }

    /// Its events, read from the metric's attribution `property`, else from the type's keys.
    pub fn matching(&self, property: Option<&str>) -> ResourceMatch {
        ResourceMatch {
            properties: property.map_or_else(|| self.keys.clone(), |p| vec![p.to_owned()]),
            value: self.key_value.clone(),
        }
    }
}

fn within(from: Option<NaiveDateTime>, until: Option<NaiveDateTime>, at: NaiveDateTime) -> bool {
    from.is_none_or(|from| from <= at) && until.is_none_or(|until| at < until)
}

/// The events of `resource` timestamped in `[from, until)`, which another subscription bills.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct UsageClaim {
    pub resource: ResourceMatch,
    pub from: NaiveDateTime,
    pub until: Option<NaiveDateTime>,
}

impl UsageClaim {
    fn covers(&self, properties: &HashMap<String, String>, at: NaiveDateTime) -> bool {
        self.resource.matches(properties) && within(Some(self.from), self.until, at)
    }
}

/// Over `[from, until)` (unbounded when absent), the events `matching` carries (all of them
/// without it), less those `excluded` claims.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ScopeSegment {
    pub from: Option<NaiveDateTime>,
    pub until: Option<NaiveDateTime>,
    pub matching: Option<ResourceMatch>,
    pub excluded: Vec<UsageClaim>,
}

/// Which of its customer's events a usage read counts: those of the segment their timestamp
/// falls in. No segment counts every event of the customer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize)]
pub struct UsageScope {
    pub segments: Vec<ScopeSegment>,
}

impl UsageScope {
    /// None of the customer's events: one empty window.
    pub fn nothing() -> Self {
        let epoch = Some(NaiveDateTime::default());
        Self::single(ScopeSegment {
            from: epoch,
            until: epoch,
            matching: None,
            excluded: Vec::new(),
        })
    }

    fn single(segment: ScopeSegment) -> Self {
        Self {
            segments: vec![segment],
        }
    }

    pub fn is_customer_wide(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn is_nothing(&self) -> bool {
        *self == Self::nothing()
    }

    /// Whether it bills a resource's events only, or none: disjoint from a customer-wide scope.
    pub fn is_resource_scoped(&self) -> bool {
        self.is_nothing() || self.segments.iter().any(|s| s.matching.is_some())
    }

    pub fn contains(&self, properties: &HashMap<String, String>, at: NaiveDateTime) -> bool {
        if self.is_customer_wide() {
            return true;
        }
        self.segments
            .iter()
            .find(|segment| within(segment.from, segment.until, at))
            .is_some_and(|segment| {
                segment
                    .matching
                    .as_ref()
                    .is_none_or(|resource| resource.matches(properties))
                    && !segment
                        .excluded
                        .iter()
                        .any(|claim| claim.covers(properties, at))
            })
    }

    /// Empty for the whole customer, else a digest of the scope, so stored state computed under
    /// one scope is never read under another.
    pub fn fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        if self.is_customer_wide() {
            return String::new();
        }
        let rendered = serde_json::to_vec(self).unwrap_or_default();
        Sha256::digest(rendered)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// The events of `metric` a limit counts on the resource of `resource_type` keyed `key_value`
/// (§3.7), whichever subscription bills them; `None`, every event of the customer.
pub fn resource_scope(
    metric: &BillableMetric,
    resource_type: &ResourceType,
    key_value: Option<&str>,
) -> UsageScope {
    let Some(key_value) = key_value else {
        return UsageScope::default();
    };
    let property = metric
        .attribution()
        .filter(|attribution| attribution.type_id == resource_type.id)
        .and_then(|attribution| attribution.property);
    UsageScope::single(ScopeSegment {
        from: None,
        until: None,
        matching: Some(
            AttributedResource::of(resource_type, key_value).matching(property.as_deref()),
        ),
        excluded: Vec::new(),
    })
}

/// A fee's metric and the instants `[from, until)` it bills it over, within its subscription's life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteredSpan {
    pub metric_id: BillableMetricId,
    pub from: NaiveDateTime,
    pub until: Option<NaiveDateTime>,
}

/// A fee added late bills and claims its metric from `until` only, though dated `from`: what
/// others invoiced over `[from, until)` stays theirs (§3.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageFloor {
    pub metric_id: BillableMetricId,
    pub from: NaiveDateTime,
    pub until: NaiveDateTime,
}

/// A span a sibling subscription for `resource` bills, and whether that sibling is older than
/// the subscription read: the older one bills an event both would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedClaim {
    pub resource: AttributedResource,
    pub span: MeteredSpan,
    pub outranks: bool,
}

/// How a subscription's usage is attributed, for any metric it bills.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubscriptionUsageScopes {
    /// Fixed once active, so its dates are its only segment; `None` for the customer.
    pub resource: Option<AttributedResource>,
    /// From when it bills its usage, when an activation after its start date set it (§3.6).
    pub from: Option<NaiveDateTime>,
    /// Up to when it bills its usage, once the decision to end it stored where (§3.6).
    pub until: Option<NaiveDateTime>,
    pub claims: Vec<RankedClaim>,
    /// What of its own resource's usage its fees added late leave to others.
    pub floors: Vec<UsageFloor>,
}

impl SubscriptionUsageScopes {
    pub fn scope_of(&self, metric: &BillableMetric) -> UsageScope {
        let Some(attribution) = metric.attribution() else {
            return match self.resource {
                Some(_) => UsageScope::nothing(),
                None => self.customer_scope(),
            };
        };
        if self
            .resource
            .as_ref()
            .is_some_and(|own| own.type_id != attribution.type_id)
        {
            return UsageScope::nothing();
        }
        let property = attribution.property.as_deref();
        let own = self.resource.as_ref();
        let mut excluded = self
            .claims
            .iter()
            .filter(|ranked| {
                ranked.span.metric_id == metric.id
                    && ranked.resource.type_id == attribution.type_id
                    && own.is_none_or(|own| {
                        ranked.outranks && ranked.resource.key_value == own.key_value
                    })
            })
            .map(|ranked| UsageClaim {
                resource: ranked.resource.matching(property),
                from: ranked.span.from,
                until: ranked.span.until,
            })
            .chain(own.into_iter().flat_map(|own| {
                self.floors
                    .iter()
                    .filter(|floor| floor.metric_id == metric.id)
                    .map(|floor| UsageClaim {
                        resource: own.matching(property),
                        from: floor.from,
                        until: Some(floor.until),
                    })
            }))
            .collect::<Vec<_>>();
        excluded.sort();
        excluded.dedup();
        if own.is_none() && excluded.is_empty() {
            return self.customer_scope();
        }
        UsageScope::single(ScopeSegment {
            from: self.from,
            until: self.until,
            matching: own.map(|own| own.matching(property)),
            excluded,
        })
    }

    /// Its resource alone, read from its type's keys, for events of no metric it prices.
    pub fn own_scope(&self) -> UsageScope {
        self.resource.as_ref().map_or_else(
            || self.customer_scope(),
            |own| {
                UsageScope::single(ScopeSegment {
                    from: self.from,
                    until: self.until,
                    matching: Some(own.matching(None)),
                    excluded: Vec::new(),
                })
            },
        )
    }

    /// Every event of the customer, from its activation when that came late, to its stored end.
    fn customer_scope(&self) -> UsageScope {
        if self.from.is_none() && self.until.is_none() {
            return UsageScope::default();
        }
        UsageScope::single(ScopeSegment {
            from: self.from,
            until: self.until,
            matching: None,
            excluded: Vec::new(),
        })
    }

    pub fn apply(&self, mut metric: BillableMetric) -> BillableMetric {
        metric.usage_scope = self.scope_of(&metric);
        metric
    }
}

/// A subscription of the customer as attribution sees it.
#[derive(Debug, Clone)]
pub struct AttributedSubscription {
    pub id: SubscriptionId,
    pub created_at: NaiveDateTime,
    /// Its life `[start, end)`: from its start to when it ended, else to its end date.
    pub start: NaiveDateTime,
    pub end: Option<NaiveDateTime>,
    pub resource: Option<AttributedResource>,
    /// What it bills once started. A subscription for a resource activated on checkout or by
    /// hand bills nothing before that: see `claims_now`.
    pub spans: Vec<MeteredSpan>,
    /// No claim start yet: it awaits a checkout or a manual activation, and claims nothing.
    pub awaiting_activation: bool,
    /// The instant an activation after its start date came: it bills its usage from then, though
    /// its first period starts earlier. Its `start` too.
    pub activated_from: Option<NaiveDateTime>,
    /// The end the decision to end it stored, which bounds what it bills itself.
    pub claimed_until: Option<NaiveDateTime>,
    /// Its fees added late, which claim their metric from later than their dates.
    pub floors: Vec<UsageFloor>,
}

impl AttributedSubscription {
    /// Whether its spans bill now. One awaiting its activation may never be activated, so until
    /// then its resource's usage stays with the customer's subscriptions.
    pub fn claims_now(&self) -> bool {
        !self.awaiting_activation
    }

    /// Who wins an event two subscriptions for one resource both bill: the older.
    pub fn outranks(&self, id: SubscriptionId, created_at: NaiveDateTime) -> bool {
        (self.created_at, self.id.as_uuid()) < (created_at, id.as_uuid())
    }
}

/// The subscription billing `metric`'s events at `at` of `resource` (its type and key value),
/// as routing picks it (§3.6): the oldest of the resource's own when the metric is attributed to
/// its type, else the oldest of the customer's own. None when no subscription bills them.
pub fn paying_subscription(
    subscriptions: &[AttributedSubscription],
    metric: &BillableMetric,
    resource: Option<(ResourceTypeId, &str)>,
    at: NaiveDateTime,
) -> Option<SubscriptionId> {
    let bills = |s: &&AttributedSubscription| {
        s.claims_now()
            && within(Some(s.start), s.end, at)
            && s.spans
                .iter()
                .any(|span| span.metric_id == metric.id && within(Some(span.from), span.until, at))
    };
    let oldest = |of: &dyn Fn(&AttributedSubscription) -> bool| {
        subscriptions
            .iter()
            .filter(bills)
            .filter(|s| of(s))
            .min_by_key(|s| (s.created_at, s.id.as_uuid()))
            .map(|s| s.id)
    };
    resource
        .filter(|(type_id, _)| metric.attributed_to_type_id == Some(*type_id))
        .and_then(|(type_id, key_value)| {
            oldest(&|s| {
                s.resource
                    .as_ref()
                    .is_some_and(|r| r.type_id == type_id && r.key_value == key_value)
            })
        })
        .or_else(|| oldest(&|s| s.resource.is_none()))
}

/// The scopes of the subscription `id` for `resource` among the customer's `subscriptions` (it
/// may be one of them): its claims are the spans of the others for a resource.
pub fn scopes_for(
    id: SubscriptionId,
    created_at: NaiveDateTime,
    resource: Option<AttributedResource>,
    subscriptions: &[AttributedSubscription],
) -> SubscriptionUsageScopes {
    let claims = subscriptions
        .iter()
        .filter(|sibling| sibling.id != id && sibling.claims_now())
        .filter_map(|sibling| sibling.resource.as_ref().map(|r| (sibling, r)))
        .flat_map(|(sibling, resource)| {
            let outranks = sibling.outranks(id, created_at);
            sibling.spans.iter().map(move |span| RankedClaim {
                resource: resource.clone(),
                span: span.clone(),
                outranks,
            })
        })
        .collect();
    let own = subscriptions
        .iter()
        .find(|subscription| subscription.id == id)
        .filter(|subscription| subscription.resource.is_some());
    SubscriptionUsageScopes {
        resource,
        from: own.and_then(|subscription| subscription.activated_from),
        until: own.and_then(|subscription| subscription.claimed_until),
        claims,
        floors: own.map_or_else(Vec::new, |subscription| subscription.floors.clone()),
    }
}

/// The events of `metric` no subscription for a resource bills.
pub fn unclaimed_scope(
    subscriptions: &[AttributedSubscription],
    metric: &BillableMetric,
) -> UsageScope {
    scopes_for(
        SubscriptionId::new(),
        NaiveDateTime::MAX,
        None,
        subscriptions,
    )
    .scope_of(metric)
}

pub const PLAN_SOLD_FOR_MISMATCH: &str = "plan_sold_for_mismatch";
pub const METRIC_NOT_ATTRIBUTED: &str = "metric_not_attributed";

/// A metric as each API names it: v1 and the dashboard by its event code, v2 by its catalog code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MetricCodes {
    pub catalog_code: String,
    pub code: String,
}

/// What a sale (a subscription, a quote, a checkout) is accepted with but worth telling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SaleWarnings {
    /// `plan_sold_for_mismatch`, `metric_not_attributed`.
    pub codes: Vec<String>,
    /// Codes of the metrics it prices that a sale for a resource never bills: attributed to the
    /// customer only, or to another type (§3.6).
    pub metrics_not_attributed: Vec<MetricCodes>,
}

impl SaleWarnings {
    pub fn new(plan_sold_for_mismatch: bool, mut metrics_not_attributed: Vec<MetricCodes>) -> Self {
        metrics_not_attributed.sort();
        metrics_not_attributed.dedup();
        let codes = plan_sold_for_mismatch
            .then_some(PLAN_SOLD_FOR_MISMATCH)
            .into_iter()
            .chain((!metrics_not_attributed.is_empty()).then_some(METRIC_NOT_ATTRIBUTED))
            .map(str::to_owned)
            .collect();
        SaleWarnings {
            codes,
            metrics_not_attributed,
        }
    }
}

/// The customer's subscriptions for the same `resource` (or the customer) that price a metric
/// of `metric_ids` over part of `[from, until)`, with those metrics: a subscription created
/// alongside them bills the same events (the customer's) or none of them (the older bills).
pub fn billing_overlaps(
    subscriptions: &[AttributedSubscription],
    resource: Option<&AttributedResource>,
    metric_ids: &[BillableMetricId],
    from: NaiveDateTime,
    until: Option<NaiveDateTime>,
) -> Vec<(SubscriptionId, Vec<BillableMetricId>)> {
    subscriptions
        .iter()
        .filter(|subscription| subscription.resource.as_ref() == resource)
        .filter_map(|subscription| {
            let mut shared = subscription
                .spans
                .iter()
                .filter(|span| {
                    metric_ids.contains(&span.metric_id)
                        && span.until.is_none_or(|end| from < end)
                        && until.is_none_or(|until| span.from < until)
                })
                .map(|span| span.metric_id)
                .collect::<Vec<_>>();
            shared.sort_by_key(|id| id.as_uuid());
            shared.dedup();
            (!shared.is_empty()).then_some((subscription.id, shared))
        })
        .collect()
}

/// A metric's usage over `[from, until)` that no subscription bills: kept, and billed to nobody.
#[derive(Debug, Clone)]
pub struct UnattributedUsage {
    pub metric: BillableMetric,
    pub from: NaiveDateTime,
    pub until: NaiveDateTime,
    pub quantity: rust_decimal::Decimal,
    /// The subscriptions for a resource pricing the metric over the stretch.
    pub subscription_ids: Vec<SubscriptionId>,
}

/// Where a metric's recent usage goes under an attribution: what each side bills, its
/// subscriptions each reading the metric's aggregation over their own events, summed over the
/// sampled customers; and the drafts a change would recompute.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributionPreview {
    pub from: NaiveDateTime,
    pub until: NaiveDateTime,
    pub sampled_customers: u32,
    /// More customers have subscriptions for a resource than were read.
    pub sampled: bool,
    /// The quantities add up to the usage (a sum or a count), so they split it in shares. Not for
    /// latest, max, min, mean or distinct count, whose per-subscription figures don't add up.
    pub additive: bool,
    pub resource_subscriptions: rust_decimal::Decimal,
    pub customer_subscriptions: rust_decimal::Decimal,
    pub not_billed: rust_decimal::Decimal,
    pub draft_invoices: u32,
}

/// The range an unattributed usage read covers when none is asked: the calendar month (UTC) of
/// `today`, dates with the end exclusive.
pub fn unattributed_default_range(
    today: chrono::NaiveDate,
) -> (chrono::NaiveDate, chrono::NaiveDate) {
    use chrono::Datelike;
    let start = today.with_day(1).unwrap_or(today);
    let end = start
        .checked_add_months(chrono::Months::new(1))
        .unwrap_or(start);
    (start, end)
}

/// A stretch over which subscriptions for a resource price a metric and no customer-level
/// subscription does: the metric's [`unclaimed_scope`] is billed to nobody there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnattributedSpan {
    pub metric_id: BillableMetricId,
    pub from: NaiveDateTime,
    pub until: NaiveDateTime,
}

pub fn unattributed_spans(
    subscriptions: &[AttributedSubscription],
    from: NaiveDateTime,
    until: NaiveDateTime,
) -> Vec<UnattributedSpan> {
    type Windows = Vec<(NaiveDateTime, NaiveDateTime)>;
    let clip = |span: &MeteredSpan| {
        let clipped = (
            span.from.max(from),
            span.until.map_or(until, |end| end.min(until)),
        );
        (clipped.0 < clipped.1).then_some(clipped)
    };
    let mut keyed: HashMap<BillableMetricId, Windows> = HashMap::new();
    let mut fallback: HashMap<BillableMetricId, Windows> = HashMap::new();
    for subscription in subscriptions.iter().filter(|s| s.claims_now()) {
        let target = match subscription.resource {
            Some(_) => &mut keyed,
            None => &mut fallback,
        };
        for span in &subscription.spans {
            if let Some(window) = clip(span) {
                target.entry(span.metric_id).or_default().push(window);
            }
        }
    }
    let mut spans = Vec::new();
    for (metric_id, intervals) in keyed {
        let covered = fallback.remove(&metric_id).unwrap_or_default();
        for (start, end) in subtract(union(intervals), &union(covered)) {
            spans.push(UnattributedSpan {
                metric_id,
                from: start,
                until: end,
            });
        }
    }
    spans.sort_by_key(|span| (span.metric_id.as_uuid(), span.from));
    spans
}

fn union(
    mut intervals: Vec<(NaiveDateTime, NaiveDateTime)>,
) -> Vec<(NaiveDateTime, NaiveDateTime)> {
    intervals.sort();
    let mut merged: Vec<(NaiveDateTime, NaiveDateTime)> = Vec::new();
    for (start, end) in intervals {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

fn subtract(
    intervals: Vec<(NaiveDateTime, NaiveDateTime)>,
    holes: &[(NaiveDateTime, NaiveDateTime)],
) -> Vec<(NaiveDateTime, NaiveDateTime)> {
    let mut left = Vec::new();
    for (mut start, end) in intervals {
        for (hole_start, hole_end) in holes {
            if *hole_end <= start || *hole_start >= end {
                continue;
            }
            if *hole_start > start {
                left.push((start, *hole_start));
            }
            start = start.max(*hole_end);
            if start >= end {
                break;
            }
        }
        if start < end {
            left.push((start, end));
        }
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::enums::BillingMetricAggregateEnum;
    use chrono::NaiveDate;
    use common_domain::ids::{ProductFamilyId, TenantId};

    fn at(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn site_type() -> ResourceType {
        ResourceType {
            id: ResourceTypeId::new(),
            tenant_id: TenantId::new(),
            code: "site".to_owned(),
            name: "Site".to_owned(),
            plural_name: "Sites".to_owned(),
            keys: vec!["site_id".to_owned(), "site".to_owned()],
            attributes: Default::default(),
            archived_at: None,
            created_at: common_clock::now(),
        }
    }

    fn metric(attributed_to: Option<&ResourceType>, property: Option<&str>) -> BillableMetric {
        BillableMetric {
            id: BillableMetricId::new(),
            name: "API calls".to_owned(),
            description: None,
            code: "api_calls".to_owned(),
            catalog_code: "api_calls".to_owned(),
            aggregation_type: BillingMetricAggregateEnum::Count,
            aggregation_key: None,
            unit_conversion_factor: None,
            unit_conversion_rounding: None,
            segmentation_matrix: None,
            filters: None,
            usage_group_key: None,
            usage_breakdown_key: None,
            unit_singular: None,
            unit_plural: None,
            created_at: at(1),
            updated_at: None,
            archived_at: None,
            tenant_id: TenantId::new(),
            product_family_id: ProductFamilyId::new(),
            product_id: None,
            attributed_to_type_id: attributed_to.map(|t| t.id),
            attribution_property: property.map(str::to_owned),
            usage_scope: UsageScope::default(),
            rated_dimensions: Vec::new(),
        }
    }

    fn subscription(
        created: u32,
        resource: Option<AttributedResource>,
        metric: &BillableMetric,
        from: u32,
        until: Option<u32>,
    ) -> AttributedSubscription {
        AttributedSubscription {
            id: SubscriptionId::new(),
            created_at: at(created),
            start: at(from),
            end: until.map(at),
            resource,
            spans: vec![MeteredSpan {
                metric_id: metric.id,
                from: at(from),
                until: until.map(at),
            }],
            awaiting_activation: false,
            activated_from: None,
            claimed_until: None,
            floors: vec![],
        }
    }

    fn props(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn scope(
        subject: &AttributedSubscription,
        all: &[AttributedSubscription],
        metric: &BillableMetric,
    ) -> UsageScope {
        scopes_for(
            subject.id,
            subject.created_at,
            subject.resource.clone(),
            all,
        )
        .scope_of(metric)
    }

    #[test]
    fn the_payer_is_the_oldest_subscription_billing_the_resource_else_the_customers() {
        let site = site_type();
        let m = metric(Some(&site), None);
        let s1 = || Some(AttributedResource::of(&site, "s1"));
        let starter = subscription(1, s1(), &m, 1, None);
        let pro = subscription(5, s1(), &m, 5, None);
        let customer = subscription(2, None, &m, 1, None);
        let all = [pro.clone(), starter.clone(), customer.clone()];
        let on = |key| Some((site.id, key));
        assert_eq!(
            paying_subscription(&all, &m, on("s1"), at(10)),
            Some(starter.id)
        );
        assert_eq!(
            paying_subscription(&all, &m, on("s2"), at(10)),
            Some(customer.id)
        );
        assert_eq!(
            paying_subscription(&all, &m, None, at(10)),
            Some(customer.id)
        );
        let customer_metric = metric(None, None);
        let unpriced = [subscription(1, s1(), &customer_metric, 1, None)];
        assert_eq!(
            paying_subscription(&unpriced, &customer_metric, on("s1"), at(10)),
            None
        );
        let ended = [subscription(1, s1(), &m, 1, Some(4)), pro.clone()];
        assert_eq!(
            paying_subscription(&ended, &m, on("s1"), at(10)),
            Some(pro.id)
        );
    }

    #[test]
    fn a_resource_subscription_bills_its_key_or_alias_and_the_customer_the_rest() {
        let site = site_type();
        let m = metric(Some(&site), None);
        let s1 = subscription(1, Some(AttributedResource::of(&site, "s1")), &m, 1, None);
        let customer = subscription(1, None, &m, 1, None);
        let all = [s1.clone(), customer.clone()];

        let (own, rest) = (scope(&s1, &all, &m), scope(&customer, &all, &m));
        for event in [props(&[("site_id", "s1")]), props(&[("site", "s1")])] {
            assert!(own.contains(&event, at(5)) && !rest.contains(&event, at(5)));
        }
        let canonical_wins = props(&[("site_id", "s2"), ("site", "s1")]);
        assert!(!own.contains(&canonical_wins, at(5)) && rest.contains(&canonical_wins, at(5)));
        for event in [props(&[]), props(&[("site_id", "s9")])] {
            assert!(!own.contains(&event, at(5)) && rest.contains(&event, at(5)));
        }
    }

    #[test]
    fn a_set_property_is_the_only_one_read() {
        let site = site_type();
        let m = metric(Some(&site), Some("origin"));
        let s1 = subscription(1, Some(AttributedResource::of(&site, "s1")), &m, 1, None);
        let own = scope(&s1, std::slice::from_ref(&s1), &m);
        assert!(own.contains(&props(&[("origin", "s1")]), at(5)));
        assert!(!own.contains(&props(&[("site_id", "s1")]), at(5)));
    }

    #[test]
    fn a_resource_subscription_never_bills_a_customer_attributed_metric() {
        let site = site_type();
        let m = metric(None, None);
        let s1 = subscription(1, Some(AttributedResource::of(&site, "s1")), &m, 1, None);
        let customer = subscription(1, None, &m, 1, None);
        let all = [s1.clone(), customer.clone()];
        assert!(scope(&s1, &all, &m).is_nothing());
        assert!(!scope(&s1, &all, &m).contains(&props(&[("site_id", "s1")]), at(5)));
        assert!(scope(&customer, &all, &m).is_customer_wide());
    }

    #[test]
    fn between_two_subscriptions_on_one_resource_the_older_bills() {
        let site = site_type();
        let m = metric(Some(&site), None);
        let older = subscription(1, Some(AttributedResource::of(&site, "s1")), &m, 1, None);
        let newer = subscription(2, Some(AttributedResource::of(&site, "s1")), &m, 1, None);
        let other = subscription(3, Some(AttributedResource::of(&site, "s2")), &m, 1, None);
        let all = [older.clone(), newer.clone(), other.clone()];
        let event = props(&[("site_id", "s1")]);
        assert!(scope(&older, &all, &m).contains(&event, at(5)));
        assert!(!scope(&newer, &all, &m).contains(&event, at(5)));
        assert!(scope(&other, &all, &m).segments[0].excluded.is_empty());
    }

    #[test]
    fn a_claim_holds_only_while_its_subscription_prices_the_metric() {
        let site = site_type();
        let m = metric(Some(&site), None);
        let s1 = subscription(
            1,
            Some(AttributedResource::of(&site, "s1")),
            &m,
            1,
            Some(15),
        );
        let customer = subscription(1, None, &m, 1, None);
        let rest = scope(&customer, &[s1, customer.clone()], &m);
        let event = props(&[("site_id", "s1")]);
        assert!(!rest.contains(&event, at(14)) && rest.contains(&event, at(15)));
    }

    #[test]
    fn the_counting_scope_reads_the_resource_whoever_bills_it() {
        let site = site_type();
        let m = metric(Some(&site), None);
        let scope = resource_scope(&m, &site, Some("s1"));
        assert!(scope.contains(&props(&[("site", "s1")]), at(5)));
        assert!(!scope.contains(&props(&[("site_id", "s10")]), at(5)));
        assert!(resource_scope(&m, &site, None).is_customer_wide());
    }

    #[test]
    fn unattributed_spans_leave_out_what_a_customer_subscription_bills() {
        let site = site_type();
        let m = metric(Some(&site), None);
        let s1 = subscription(1, Some(AttributedResource::of(&site, "s1")), &m, 1, None);
        let customer = subscription(1, None, &m, 10, Some(20));
        let all = [s1, customer];
        let spans = unattributed_spans(&all, at(1), at(30));
        assert_eq!(
            spans
                .iter()
                .map(|span| (span.from, span.until))
                .collect::<Vec<_>>(),
            vec![(at(1), at(10)), (at(20), at(30))]
        );
        assert_eq!(unclaimed_scope(&all, &m).segments[0].excluded.len(), 1);
    }

    #[test]
    fn only_a_subscription_for_the_same_target_overlaps() {
        let site = site_type();
        let m = metric(Some(&site), None);
        let s1 = AttributedResource::of(&site, "s1");
        let customer = subscription(1, None, &m, 1, None);
        let on_s1 = subscription(1, Some(s1.clone()), &m, 1, None);
        let ended = subscription(1, None, &m, 1, Some(5));
        let all = [customer.clone(), on_s1.clone(), ended];
        assert_eq!(
            billing_overlaps(&all, None, &[m.id], at(10), None),
            vec![(customer.id, vec![m.id])]
        );
        assert_eq!(
            billing_overlaps(&all, Some(&s1), &[m.id], at(10), None),
            vec![(on_s1.id, vec![m.id])]
        );
    }

    #[test]
    fn an_empty_key_falls_through_to_the_next() {
        let filter = ResourceMatch {
            properties: vec!["site_id".to_owned(), "site".to_owned()],
            value: "s1".to_owned(),
        };
        let props = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect::<HashMap<_, _>>()
        };
        assert!(filter.matches(&props(&[("site_id", ""), ("site", "s1")])));
        assert!(!filter.matches(&props(&[("site_id", "s2"), ("site", "s1")])));
    }
}
