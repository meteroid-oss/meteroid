use crate::StoreResult;
use crate::domain::BillableMetric;
use crate::errors::StoreError;
use chrono::NaiveDateTime;
use common_domain::ids::{BaseId, BillableMetricId, SubscriptionId};
use error_stack::bail;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_USAGE_FILTER_LEN: usize = 256;

/// The event property and value that mark the events a subscription bills, among those of its
/// customer (a usage filter). A subscription without one bills the events no filter claims.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UsageFilter {
    pub property: String,
    pub value: String,
}

impl UsageFilter {
    pub fn new(property: String, value: String) -> StoreResult<Self> {
        for (name, field) in [("property", &property), ("value", &value)] {
            if field.trim().is_empty() {
                bail!(StoreError::InvalidArgument(format!(
                    "the usage filter {name} must not be empty"
                )));
            }
            if field.trim() != field {
                bail!(StoreError::InvalidArgument(format!(
                    "the usage filter {name} must not start or end with whitespace"
                )));
            }
            if field.len() > MAX_USAGE_FILTER_LEN {
                bail!(StoreError::InvalidArgument(format!(
                    "the usage filter {name} must be at most {MAX_USAGE_FILTER_LEN} bytes"
                )));
            }
        }
        Ok(Self { property, value })
    }

    /// Both columns set or neither; anything else is a corrupt row.
    pub fn from_columns(
        property: Option<String>,
        value: Option<String>,
    ) -> StoreResult<Option<Self>> {
        match (property, value) {
            (Some(property), Some(value)) => Ok(Some(Self { property, value })),
            (None, None) => Ok(None),
            _ => bail!(StoreError::InvalidArgument(
                "a usage filter needs both a property and a value".to_string()
            )),
        }
    }

    pub fn matches(&self, properties: &HashMap<String, String>) -> bool {
        properties.get(&self.property) == Some(&self.value)
    }
}

fn within(from: Option<NaiveDateTime>, until: Option<NaiveDateTime>, at: NaiveDateTime) -> bool {
    from.is_none_or(|from| from <= at) && until.is_none_or(|until| at < until)
}

fn overlaps(
    a: (Option<NaiveDateTime>, Option<NaiveDateTime>),
    b: (Option<NaiveDateTime>, Option<NaiveDateTime>),
) -> bool {
    let starts_before_b_ends = match (a.0, b.1) {
        (Some(start), Some(end)) => start < end,
        _ => true,
    };
    let ends_after_b_starts = match (a.1, b.0) {
        (Some(end), Some(start)) => end > start,
        _ => true,
    };
    starts_before_b_ends && ends_after_b_starts
}

/// The events of `filter` timestamped in `[from, until)`, which another subscription bills.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct UsageClaim {
    pub filter: UsageFilter,
    pub from: NaiveDateTime,
    pub until: Option<NaiveDateTime>,
}

impl UsageClaim {
    fn covers(&self, properties: &HashMap<String, String>, at: NaiveDateTime) -> bool {
        self.filter.matches(properties) && within(Some(self.from), self.until, at)
    }

    fn sort_key(&self) -> (&str, &str, NaiveDateTime, Option<NaiveDateTime>) {
        (
            &self.filter.property,
            &self.filter.value,
            self.from,
            self.until,
        )
    }
}

/// Over `[from, until)` (unbounded when absent), the events `matching` carries (all of them
/// without it), less those `excluded` claims.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ScopeSegment {
    pub from: Option<NaiveDateTime>,
    pub until: Option<NaiveDateTime>,
    pub matching: Option<UsageFilter>,
    pub excluded: Vec<UsageClaim>,
}

/// Which of its customer's events a usage read counts: those of the segment their timestamp
/// falls in. No segment counts every event of the customer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize)]
pub struct UsageScope {
    pub segments: Vec<ScopeSegment>,
}

impl UsageScope {
    pub fn is_customer_wide(&self) -> bool {
        self.segments.is_empty()
    }

    /// Whether some stretch of it bills only the events of a usage filter.
    pub fn is_filtered(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| segment.matching.is_some())
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
                    .is_none_or(|filter| filter.matches(properties))
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

/// A subscription's usage filter from `from` until `until`; none bills without a filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageFilterVersion {
    pub filter: Option<UsageFilter>,
    pub from: NaiveDateTime,
    pub until: Option<NaiveDateTime>,
}

/// `versions`, in order, with the stretches between them (no filter) filled in, covering all time.
fn timeline(
    versions: &[UsageFilterVersion],
) -> Vec<(
    Option<NaiveDateTime>,
    Option<NaiveDateTime>,
    Option<&UsageFilter>,
)> {
    let mut stretches = Vec::new();
    let mut previous_end: Option<NaiveDateTime> = None;
    let mut first = true;
    for version in versions {
        if first || previous_end.is_some_and(|end| end < version.from) {
            stretches.push((previous_end, Some(version.from), None));
        }
        stretches.push((Some(version.from), version.until, version.filter.as_ref()));
        first = false;
        match version.until {
            Some(until) => previous_end = Some(until),
            None => return stretches,
        }
    }
    stretches.push((previous_end, None, None));
    stretches
}

/// A claim of a sibling subscription, and whether that sibling is older than the subscription
/// read: the older one bills an event two filters match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedClaim {
    pub claim: UsageClaim,
    pub outranks: bool,
}

/// How a subscription's usage is attributed, for any metric it bills.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubscriptionUsageScopes {
    pub versions: Vec<UsageFilterVersion>,
    pub claims: HashMap<BillableMetricId, Vec<RankedClaim>>,
}

impl SubscriptionUsageScopes {
    pub fn scope_of(&self, metric_id: BillableMetricId) -> UsageScope {
        self.scope_with(self.claims.get(&metric_id).map_or(&[][..], Vec::as_slice))
    }

    /// Its own filter alone, for events of no metric it prices.
    pub fn own_scope(&self) -> UsageScope {
        self.scope_with(&[])
    }

    fn scope_with(&self, claims: &[RankedClaim]) -> UsageScope {
        let mut segments: Vec<ScopeSegment> = Vec::new();
        for (from, until, filter) in timeline(&self.versions) {
            let excluded = claims
                .iter()
                .filter(|ranked| {
                    overlaps((from, until), (Some(ranked.claim.from), ranked.claim.until))
                })
                .filter(|ranked| match filter {
                    None => true,
                    Some(own) => {
                        let disjoint = own.property == ranked.claim.filter.property
                            && own.value != ranked.claim.filter.value;
                        ranked.outranks && !disjoint
                    }
                })
                .map(|ranked| ranked.claim.clone())
                .collect::<Vec<_>>();
            let segment = ScopeSegment {
                from,
                until,
                matching: filter.cloned(),
                excluded,
            };
            match segments.last_mut() {
                Some(last)
                    if last.matching == segment.matching && last.excluded == segment.excluded =>
                {
                    last.until = segment.until;
                }
                _ => segments.push(segment),
            }
        }
        if segments
            .iter()
            .all(|segment| segment.matching.is_none() && segment.excluded.is_empty())
        {
            return UsageScope::default();
        }
        UsageScope { segments }
    }

    pub fn apply(&self, mut metric: BillableMetric) -> BillableMetric {
        metric.usage_scope = self.scope_of(metric.id);
        metric
    }
}

/// A fee's metric and the instants `[from, until)` it bills it over, within its subscription's life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteredSpan {
    pub metric_id: BillableMetricId,
    pub from: NaiveDateTime,
    pub until: Option<NaiveDateTime>,
}

/// A subscription of the customer as attribution sees it.
#[derive(Debug, Clone)]
pub struct AttributedSubscription {
    pub id: SubscriptionId,
    pub created_at: NaiveDateTime,
    /// Its life `[start, end)`: from its start to when it ended, else to its end date.
    pub start: NaiveDateTime,
    pub end: Option<NaiveDateTime>,
    /// What it has invoiced its usage up to: the start of its current period, or its end.
    pub invoiced_until: NaiveDateTime,
    pub versions: Vec<UsageFilterVersion>,
    pub spans: Vec<MeteredSpan>,
}

impl AttributedSubscription {
    /// Who wins an event two filters both match: the older subscription.
    pub fn outranks(&self, id: SubscriptionId, created_at: NaiveDateTime) -> bool {
        (self.created_at, self.id.as_uuid()) < (created_at, id.as_uuid())
    }

    /// The stretches of its life under each filter (none: billed without one).
    pub fn stretches(&self) -> Vec<(NaiveDateTime, Option<NaiveDateTime>, Option<&UsageFilter>)> {
        timeline(&self.versions)
            .into_iter()
            .filter_map(|(from, until, filter)| {
                let from = from.map_or(self.start, |from| from.max(self.start));
                let until = match (until, self.end) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                until
                    .is_none_or(|until| from < until)
                    .then_some((from, until, filter))
            })
            .collect()
    }

    /// Where its fees bill `filter`ed (or unfiltered, with `None`) usage, per metric.
    fn billed(
        &self,
        filtered: bool,
    ) -> Vec<(
        BillableMetricId,
        Option<&UsageFilter>,
        NaiveDateTime,
        Option<NaiveDateTime>,
    )> {
        let mut billed = Vec::new();
        for (from, until, filter) in self.stretches() {
            if filter.is_some() != filtered {
                continue;
            }
            for span in &self.spans {
                let start = from.max(span.from);
                let end = match (until, span.until) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
                if end.is_none_or(|end| start < end) {
                    billed.push((span.metric_id, filter, start, end));
                }
            }
        }
        billed
    }

    fn claims(&self) -> Vec<(BillableMetricId, UsageClaim)> {
        self.billed(true)
            .into_iter()
            .filter_map(|(metric_id, filter, from, until)| {
                filter.map(|filter| {
                    (
                        metric_id,
                        UsageClaim {
                            filter: filter.clone(),
                            from,
                            until,
                        },
                    )
                })
            })
            .collect()
    }
}

fn sorted(mut claims: Vec<UsageClaim>) -> Vec<UsageClaim> {
    claims.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
    claims.dedup();
    claims
}

/// The scopes of the subscription `id` with filter `versions` among the customer's
/// `subscriptions` (it may be one of them). Without a filter it bills what no filter of the others
/// claims; with one, what its filter matches and no older filtered subscription claims.
pub fn scopes_for(
    id: SubscriptionId,
    created_at: NaiveDateTime,
    versions: &[UsageFilterVersion],
    subscriptions: &[AttributedSubscription],
) -> SubscriptionUsageScopes {
    let mut claims: HashMap<BillableMetricId, Vec<RankedClaim>> = HashMap::new();
    for sibling in subscriptions.iter().filter(|sibling| sibling.id != id) {
        let outranks = sibling.outranks(id, created_at);
        for (metric_id, claim) in sibling.claims() {
            let entry = claims.entry(metric_id).or_default();
            let ranked = RankedClaim { claim, outranks };
            if !entry.contains(&ranked) {
                entry.push(ranked);
            }
        }
    }
    for list in claims.values_mut() {
        list.sort_by(|a, b| {
            (a.claim.sort_key(), a.outranks).cmp(&(b.claim.sort_key(), b.outranks))
        });
    }
    SubscriptionUsageScopes {
        versions: versions.to_vec(),
        claims,
    }
}

/// The customer's subscriptions that bill, without a usage filter, a metric of `metric_ids` over
/// part of `[from, until)`, with the metrics they share: an unfiltered subscription created
/// alongside them would bill the same events (both bill).
pub fn unfiltered_overlaps(
    subscriptions: &[AttributedSubscription],
    metric_ids: &[BillableMetricId],
    from: NaiveDateTime,
    until: Option<NaiveDateTime>,
) -> Vec<(SubscriptionId, Vec<BillableMetricId>)> {
    subscriptions
        .iter()
        .filter_map(|subscription| {
            let mut shared = subscription
                .billed(false)
                .into_iter()
                .filter(|(metric_id, _, start, end)| {
                    metric_ids.contains(metric_id)
                        && end.is_none_or(|end| from < end)
                        && until.is_none_or(|until| *start < until)
                })
                .map(|(metric_id, ..)| metric_id)
                .collect::<Vec<_>>();
            shared.sort_by_key(|id| id.as_uuid());
            shared.dedup();
            (!shared.is_empty()).then_some((subscription.id, shared))
        })
        .collect()
}

/// A metric's usage over `[from, until)` that matches no subscription's usage filter while no
/// subscription without one bills the metric: kept, and billed to nobody.
#[derive(Debug, Clone)]
pub struct UnattributedUsage {
    pub metric: BillableMetric,
    pub from: NaiveDateTime,
    pub until: NaiveDateTime,
    pub quantity: rust_decimal::Decimal,
    /// The subscriptions pricing the metric over the stretch, whose usage filters leave it out.
    pub subscription_ids: Vec<SubscriptionId>,
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

/// A stretch of time over which a metric's events no unfiltered subscription bills, and the
/// claims of the filtered ones; what they leave is usage billed to nobody.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnattributedSpan {
    pub metric_id: BillableMetricId,
    pub from: NaiveDateTime,
    pub until: NaiveDateTime,
    pub scope: UsageScope,
}

/// Where in `[from, until)` a metric some filtered subscription bills has no unfiltered
/// subscription to fall back on.
pub fn unattributed_spans(
    subscriptions: &[AttributedSubscription],
    from: NaiveDateTime,
    until: NaiveDateTime,
) -> Vec<UnattributedSpan> {
    type Windows = Vec<(NaiveDateTime, NaiveDateTime)>;
    let clip = |start: NaiveDateTime, end: Option<NaiveDateTime>| {
        let clipped = (start.max(from), end.map_or(until, |end| end.min(until)));
        (clipped.0 < clipped.1).then_some(clipped)
    };
    let mut keyed: HashMap<BillableMetricId, Windows> = HashMap::new();
    let mut fallback: HashMap<BillableMetricId, Windows> = HashMap::new();
    let mut claims: HashMap<BillableMetricId, Vec<UsageClaim>> = HashMap::new();
    for subscription in subscriptions {
        for (metric_id, _, start, end) in subscription.billed(false) {
            if let Some(window) = clip(start, end) {
                fallback.entry(metric_id).or_default().push(window);
            }
        }
        for (metric_id, claim) in subscription.claims() {
            if let Some(window) = clip(claim.from, claim.until) {
                keyed.entry(metric_id).or_default().push(window);
            }
            claims.entry(metric_id).or_default().push(claim);
        }
    }
    let mut spans = Vec::new();
    for (metric_id, intervals) in keyed {
        let covered = fallback.remove(&metric_id).unwrap_or_default();
        let excluded = sorted(claims.remove(&metric_id).unwrap_or_default());
        for (start, end) in subtract(union(intervals), &union(covered)) {
            spans.push(UnattributedSpan {
                metric_id,
                from: start,
                until: end,
                scope: UsageScope {
                    segments: vec![ScopeSegment {
                        from: None,
                        until: None,
                        matching: None,
                        excluded: excluded.clone(),
                    }],
                },
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
    use chrono::NaiveDate;

    fn at(day: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn filter(property: &str, value: &str) -> UsageFilter {
        UsageFilter::new(property.to_owned(), value.to_owned()).unwrap()
    }

    fn version(filter: Option<UsageFilter>, from: u32, until: Option<u32>) -> UsageFilterVersion {
        UsageFilterVersion {
            filter,
            from: at(from),
            until: until.map(at),
        }
    }

    fn subscription(
        created: u32,
        versions: Vec<UsageFilterVersion>,
        metric_id: BillableMetricId,
        from: u32,
        until: Option<u32>,
    ) -> AttributedSubscription {
        AttributedSubscription {
            id: SubscriptionId::new(),
            created_at: at(created),
            start: at(from),
            end: until.map(at),
            invoiced_until: at(from),
            versions,
            spans: vec![MeteredSpan {
                metric_id,
                from: at(from),
                until: until.map(at),
            }],
        }
    }

    fn keyed(created: u32, value: &str, metric_id: BillableMetricId) -> AttributedSubscription {
        subscription(
            created,
            vec![version(Some(filter("api_key", value)), 1, None)],
            metric_id,
            1,
            None,
        )
    }

    #[test]
    fn only_an_unfiltered_subscription_sharing_a_metric_over_the_period_overlaps() {
        let (m1, m2) = (BillableMetricId::new(), BillableMetricId::new());
        let unfiltered = subscription(1, vec![], m1, 1, None);
        let other_metric = subscription(1, vec![], m2, 1, None);
        let ended = subscription(1, vec![], m1, 1, Some(5));
        let filtered = keyed(1, "a", m1);
        let all = [unfiltered.clone(), other_metric, ended, filtered];
        assert_eq!(
            unfiltered_overlaps(&all, &[m1], at(10), None),
            vec![(unfiltered.id, vec![m1])]
        );
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
        metric: BillableMetricId,
    ) -> UsageScope {
        scopes_for(subject.id, subject.created_at, &subject.versions, all).scope_of(metric)
    }

    #[test]
    fn the_unfiltered_subscription_bills_what_no_filter_claims() {
        let metric = BillableMetricId::new();
        let a = keyed(1, "key-A", metric);
        let b = subscription(1, vec![], metric, 1, None);
        let all = [a.clone(), b.clone()];

        let scope_a = scope(&a, &all, metric);
        let scope_b = scope(&b, &all, metric);
        let e1 = props(&[("api_key", "key-A")]);
        let e2 = props(&[("api_key", "key-Z")]);
        let e3 = props(&[]);
        assert!(scope_a.contains(&e1, at(5)));
        assert!(!scope_a.contains(&e2, at(5)) && !scope_a.contains(&e3, at(5)));
        assert!(!scope_b.contains(&e1, at(5)));
        assert!(scope_b.contains(&e2, at(5)) && scope_b.contains(&e3, at(5)));
        assert!(scope(&b, &all, BillableMetricId::new()).is_customer_wide());
    }

    #[test]
    fn a_claim_holds_only_while_its_subscription_lives() {
        let metric = BillableMetricId::new();
        let a = subscription(
            1,
            vec![version(Some(filter("api_key", "key-A")), 1, None)],
            metric,
            1,
            Some(15),
        );
        let b = subscription(1, vec![], metric, 1, None);
        let scope_b = scope(&b, &[a, b.clone()], metric);
        let e1 = props(&[("api_key", "key-A")]);
        assert!(!scope_b.contains(&e1, at(14)));
        assert!(scope_b.contains(&e1, at(15)));
    }

    #[test]
    fn a_filter_applies_from_its_version_on() {
        let metric = BillableMetricId::new();
        let a = subscription(
            1,
            vec![
                version(Some(filter("api_key", "key-A")), 1, Some(10)),
                version(Some(filter("api_key", "key-B")), 10, None),
            ],
            metric,
            1,
            None,
        );
        let b = subscription(1, vec![], metric, 1, None);
        let all = [a.clone(), b.clone()];
        let key_a = props(&[("api_key", "key-A")]);
        let key_b = props(&[("api_key", "key-B")]);

        let scope_a = scope(&a, &all, metric);
        assert!(scope_a.contains(&key_a, at(9)) && !scope_a.contains(&key_b, at(9)));
        assert!(!scope_a.contains(&key_a, at(10)) && scope_a.contains(&key_b, at(10)));
        let scope_b = scope(&b, &all, metric);
        assert!(scope_b.contains(&key_b, at(9)) && !scope_b.contains(&key_a, at(9)));
        assert!(scope_b.contains(&key_a, at(10)) && !scope_b.contains(&key_b, at(10)));
    }

    #[test]
    fn a_removed_filter_bills_without_one_from_then_on() {
        let metric = BillableMetricId::new();
        let a = subscription(
            1,
            vec![
                version(Some(filter("api_key", "key-A")), 1, Some(10)),
                version(None, 10, None),
            ],
            metric,
            1,
            None,
        );
        let scope_a = scope(&a, std::slice::from_ref(&a), metric);
        let key_z = props(&[("api_key", "key-Z")]);
        assert!(!scope_a.contains(&key_z, at(9)));
        assert!(scope_a.contains(&key_z, at(10)));
    }

    #[test]
    fn an_event_two_filters_match_goes_to_the_older_subscription() {
        let metric = BillableMetricId::new();
        let older = keyed(1, "key-A", metric);
        let newer = subscription(
            2,
            vec![version(Some(filter("project", "p1")), 1, None)],
            metric,
            1,
            None,
        );
        let same_property = keyed(3, "key-B", metric);
        let all = [older.clone(), newer.clone(), same_property.clone()];
        let both = props(&[("api_key", "key-A"), ("project", "p1")]);

        assert!(scope(&older, &all, metric).contains(&both, at(5)));
        let newer_scope = scope(&newer, &all, metric);
        assert!(!newer_scope.contains(&both, at(5)));
        assert!(newer_scope.contains(&props(&[("project", "p1")]), at(5)));
        let disjoint = scope(&same_property, &all, metric);
        assert_eq!(disjoint.segments.len(), 2);
        assert_eq!(disjoint.segments[1].excluded.len(), 1);
        assert_eq!(
            disjoint.segments[1].excluded[0].filter,
            filter("project", "p1")
        );
    }

    #[test]
    fn unattributed_spans_leave_out_what_an_unfiltered_subscription_bills() {
        let metric = BillableMetricId::new();
        let a = keyed(1, "key-A", metric);
        let b = subscription(1, vec![], metric, 10, Some(20));
        let spans = unattributed_spans(&[a, b], at(1), at(30));
        assert_eq!(
            spans
                .iter()
                .map(|span| (span.from, span.until))
                .collect::<Vec<_>>(),
            vec![(at(1), at(10)), (at(20), at(30))]
        );
        assert!(
            spans
                .iter()
                .all(|span| span.scope.segments[0].excluded.len() == 1)
        );
    }

    #[test]
    fn a_filter_needs_a_property_and_a_value() {
        assert!(UsageFilter::new(String::new(), "v".to_owned()).is_err());
        assert!(UsageFilter::new("p".to_owned(), " v".to_owned()).is_err());
        assert!(UsageFilter::from_columns(Some("p".to_owned()), None).is_err());
        assert_eq!(UsageFilter::from_columns(None, None).unwrap(), None);
    }
}
