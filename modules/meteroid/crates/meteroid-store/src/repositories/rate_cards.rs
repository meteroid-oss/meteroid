//! The tenant's rate cards and their dated entries. Rates are set in bulk from one day, as a
//! provider's price feed syncs them or a user edits the card: a repeated sync changes nothing.
//! Each write is in the card's activity, with who made it.
use crate::StoreResult;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::rate_cards::{
    RateCard, RateCardDenomination, RateCardEntry, RateCardNew, RateCardRatesSet, RateCardUpsert,
    plan_upsert, rates_changed,
};
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::{BaseId, BillableMetricId, RateCardId, TenantId};
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::credit_units::CreditUnitRow;
use diesel_models::errors::DatabaseError;
use diesel_models::rate_cards::{RateCardEntryRow, RateCardRow, RateCardRowNew};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

#[async_trait::async_trait]
pub trait RateCardsInterface {
    async fn create_rate_card(&self, actor: Actor, new: RateCardNew) -> StoreResult<RateCard>;

    /// Renames the card and sets what its unmatched usage does; a field left `None` stays. What it
    /// rates (metric, dimensions, denomination, unit size) never changes.
    async fn update_rate_card(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: RateCardId,
        name: Option<String>,
        on_unmatched: Option<crate::domain::rate_cards::RateCardUnmatched>,
    ) -> StoreResult<RateCard>;

    /// Ordered by code, archived cards left out; only the metric's cards when one is given.
    async fn list_rate_cards(
        &self,
        tenant_id: TenantId,
        metric_id: Option<BillableMetricId>,
    ) -> StoreResult<Vec<RateCard>>;

    /// The tenant's cards after `after` in id order, at most `limit`: a keyset page of API v2.
    async fn list_rate_cards_after(
        &self,
        tenant_id: TenantId,
        metric_id: Option<BillableMetricId>,
        include_archived: bool,
        after: Option<RateCardId>,
        limit: usize,
    ) -> StoreResult<Vec<RateCard>>;

    /// The card and all its entries, those scheduled included.
    async fn get_rate_card(
        &self,
        tenant_id: TenantId,
        id: RateCardId,
    ) -> StoreResult<(RateCard, Vec<RateCardEntry>)>;

    /// The card, without its entries.
    async fn find_rate_card(&self, tenant_id: TenantId, id: RateCardId) -> StoreResult<RateCard>;

    /// Off the catalog for new prices (`true`) or back on it (`false`), recorded in its activity
    /// when it changes; the prices it rates keep rating, and its rates can still be set.
    async fn set_rate_card_archived(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: RateCardId,
        archived: bool,
    ) -> StoreResult<RateCard>;

    /// The id of the tenant's card of `code`, archived ones included.
    async fn find_rate_card_id_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<RateCardId>;

    /// Sets rates and removes keys, all from the batch's day (R-131). A batch changing anything
    /// is recorded, each rate with the one it follows and each scheduled entry it deletes.
    async fn set_rate_card_rates(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: RateCardId,
        set: RateCardRatesSet,
        today: chrono::NaiveDate,
    ) -> StoreResult<RateCardUpsert>;
}

fn not_found(id: RateCardId) -> Report<StoreError> {
    Report::new(StoreError::ValueNotFound(format!("rate card {id}")))
}

#[async_trait::async_trait]
impl RateCardsInterface for Store {
    async fn update_rate_card(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: RateCardId,
        name: Option<String>,
        on_unmatched: Option<crate::domain::rate_cards::RateCardUnmatched>,
    ) -> StoreResult<RateCard> {
        let name = name.map(|name| name.trim().to_owned());
        if name.as_deref().is_some_and(str::is_empty) {
            return Err(Report::new(StoreError::InvalidArgument(
                "a rate card has a name".to_owned(),
            )));
        }
        self.transaction(|conn| {
            let (actor, name) = (&actor, name.clone());
            async move {
                let row = RateCardRow::update_terms(
                    conn,
                    tenant_id,
                    id,
                    name,
                    on_unmatched.map(Into::into),
                )
                .await?
                .ok_or_else(|| not_found(id))?;
                let card: RateCard = row.try_into()?;
                let activity = Activity::new(
                    ActivityType::EntityUpdated,
                    EntityType::RateCard,
                    card.id.as_uuid(),
                )
                .with_metadata(serde_json::json!({
                    "name": card.name,
                }));
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;
                Ok(card)
            }
            .scope_boxed()
        })
        .await
    }

    async fn create_rate_card(&self, actor: Actor, new: RateCardNew) -> StoreResult<RateCard> {
        let mut dimensions = new.dimensions.clone();
        dimensions.sort();
        dimensions.dedup();
        if new.dimensions.is_empty()
            || new.dimensions.len() > 4
            || dimensions.len() != new.dimensions.len()
            || new.dimensions.iter().any(|d| d.trim().is_empty())
        {
            return Err(Report::new(StoreError::InvalidArgument(
                "a rate card rates by one to four distinct dimensions".to_owned(),
            )));
        }
        if new.unit_size <= rust_decimal::Decimal::ZERO {
            return Err(Report::new(StoreError::InvalidArgument(
                "a rate card's unit size is positive".to_owned(),
            )));
        }
        let mut conn = self.get_conn().await?;
        // A metric's declared dimension values only suggest keys: a card rates any metric.
        let metric = BillableMetricRow::find_by_id(&mut conn, new.metric_id, new.tenant_id).await?;
        if !crate::repositories::price_components::adds_up(metric.aggregation_type) {
            return Err(Report::new(StoreError::InvalidArgument(
                "a rate card rates a metric that adds up: a sum or a count".to_owned(),
            )));
        }
        let credit_unit_id = match &new.denomination {
            RateCardDenomination::Money => None,
            RateCardDenomination::CreditUnit(unit) => {
                CreditUnitRow::find_by_id(&mut conn, new.tenant_id, *unit)
                    .await?
                    .ok_or_else(|| {
                        Report::new(StoreError::ValueNotFound(format!("credit unit {unit}")))
                    })?;
                Some(*unit)
            }
        };
        drop(conn);
        let row_new = RateCardRowNew {
            id: RateCardId::new(),
            tenant_id: new.tenant_id,
            code: new.code.clone(),
            name: new.name,
            metric_id: new.metric_id,
            dimensions: new.dimensions.into_iter().map(Some).collect(),
            credit_unit_id,
            on_unmatched: new.on_unmatched.into(),
            unit_size: new.unit_size,
        };
        self.transaction(|conn| {
            let (actor, row_new) = (&actor, &row_new);
            async move {
                let row = row_new.insert(conn).await.map_err(|err| {
                    if matches!(err.error.current_context(), DatabaseError::UniqueViolation) {
                        Report::new(StoreError::DuplicateValue {
                            entity: "rate card",
                            key: Some(row_new.code.clone()),
                        })
                    } else {
                        err.into()
                    }
                })?;
                let card: RateCard = row.try_into()?;
                let activity = Activity::new(
                    ActivityType::RateCardCreated,
                    EntityType::RateCard,
                    card.id.as_uuid(),
                )
                .with_metadata(serde_json::json!({
                    "code": card.code,
                    "name": card.name,
                    "dimensions": card.dimensions,
                    "unit_size": card.unit_size.normalize().to_string(),
                }));
                self.internal
                    .record_audit_tx(conn, card.tenant_id, actor, AuditInput::Activity(activity))
                    .await?;
                Ok(card)
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_rate_cards(
        &self,
        tenant_id: TenantId,
        metric_id: Option<BillableMetricId>,
    ) -> StoreResult<Vec<RateCard>> {
        let mut conn = self.get_conn().await?;
        RateCardRow::list(&mut conn, tenant_id, metric_id)
            .await?
            .into_iter()
            .map(|row| row.try_into().map_err(Report::new))
            .collect()
    }

    async fn list_rate_cards_after(
        &self,
        tenant_id: TenantId,
        metric_id: Option<BillableMetricId>,
        include_archived: bool,
        after: Option<RateCardId>,
        limit: usize,
    ) -> StoreResult<Vec<RateCard>> {
        let mut conn = self.get_conn().await?;
        RateCardRow::list_page(
            &mut conn,
            tenant_id,
            metric_id,
            include_archived,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await?
        .into_iter()
        .map(|row| row.try_into().map_err(Report::new))
        .collect()
    }

    async fn get_rate_card(
        &self,
        tenant_id: TenantId,
        id: RateCardId,
    ) -> StoreResult<(RateCard, Vec<RateCardEntry>)> {
        let mut conn = self.get_conn().await?;
        let card: RateCard = RateCardRow::find(&mut conn, tenant_id, id)
            .await?
            .ok_or_else(|| not_found(id))?
            .try_into()?;
        let entries = RateCardEntryRow::list_by_card(&mut conn, tenant_id, id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok((card, entries))
    }

    async fn find_rate_card(&self, tenant_id: TenantId, id: RateCardId) -> StoreResult<RateCard> {
        let mut conn = self.get_conn().await?;
        Ok(RateCardRow::find(&mut conn, tenant_id, id)
            .await?
            .ok_or_else(|| not_found(id))?
            .try_into()?)
    }

    async fn set_rate_card_archived(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: RateCardId,
        archived: bool,
    ) -> StoreResult<RateCard> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let current = RateCardRow::find_for_update(conn, tenant_id, id)
                    .await?
                    .ok_or_else(|| not_found(id))?;
                if current.archived_at.is_some() == archived {
                    let card: RateCard = current.try_into()?;
                    return Ok(card);
                }
                let row = RateCardRow::set_archived(
                    conn,
                    tenant_id,
                    id,
                    archived.then(common_clock::now),
                )
                .await?
                .ok_or_else(|| not_found(id))?;
                let card: RateCard = row.try_into()?;
                let activity = Activity::new(
                    ActivityType::EntityUpdated,
                    EntityType::RateCard,
                    card.id.as_uuid(),
                )
                .with_metadata(serde_json::json!({
                    "archived": archived,
                }));
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;
                Ok(card)
            }
            .scope_boxed()
        })
        .await
    }

    async fn find_rate_card_id_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<RateCardId> {
        let mut conn = self.get_conn().await?;
        RateCardRow::find_by_code(&mut conn, tenant_id, code)
            .await?
            .map(|row| row.id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("rate card {code}"))))
    }

    async fn set_rate_card_rates(
        &self,
        actor: Actor,
        tenant_id: TenantId,
        id: RateCardId,
        set: RateCardRatesSet,
        today: chrono::NaiveDate,
    ) -> StoreResult<RateCardUpsert> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let card: RateCard = RateCardRow::find_for_update(conn, tenant_id, id)
                    .await?
                    .ok_or_else(|| not_found(id))?
                    .try_into()?;
                let existing: Vec<RateCardEntry> =
                    RateCardEntryRow::list_by_card(conn, tenant_id, id)
                        .await?
                        .into_iter()
                        .map(Into::into)
                        .collect();
                let effective_from = set.effective_from.unwrap_or(today);
                let inputs = set.inputs(&card, today)?;
                let (inserts, replaced, summary) = plan_upsert(&card, &existing, &inputs, today)?;
                // The deleted scheduled entries live on in the audit only.
                let changed = rates_changed(&existing, &inserts, &replaced, effective_from);
                RateCardEntryRow::delete_scheduled(conn, tenant_id, &replaced, today).await?;
                let now = common_clock::now();
                let rows: Vec<RateCardEntryRow> = inserts
                    .into_iter()
                    .map(|entry| RateCardEntryRow {
                        id: entry.id,
                        tenant_id,
                        rate_card_id: id,
                        key: entry.key.iter().map(|p| Some(p.as_text())).collect(),
                        effective_from: entry.effective_from,
                        rate: entry.rate,
                        label: entry.label,
                        created_at: now,
                        currency: entry.currency,
                    })
                    .collect();
                RateCardEntryRow::insert_batch(conn, &rows).await?;
                if !changed.is_empty() {
                    let metadata = serde_json::to_value(&changed).map_err(|e| {
                        StoreError::SerdeError("Failed to serialize rate card changes".into(), e)
                    })?;
                    let activity = Activity::new(
                        ActivityType::RateCardRatesChanged,
                        EntityType::RateCard,
                        id.as_uuid(),
                    )
                    .with_metadata(metadata);
                    self.internal
                        .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;
                }
                if summary.inserted + summary.removed + summary.replaced > 0 {
                    diesel_models::billing_controller::BillingControllerRow::retry_failing(
                        conn, tenant_id,
                    )
                    .await?;
                }
                Ok(summary)
            }
            .scope_boxed()
        })
        .await
    }
}
