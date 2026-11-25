use std::collections::BTreeMap;
use std::str::FromStr;

use chrono::NaiveDate;
use common_domain::ids::{BaseId, BillableMetricId, CreditUnitId, RateCardId};
use common_grpc::middleware::server::auth::RequestExt;
use error_stack::Report;
use meteroid_grpc::meteroid::api::ratecards::v1::{
    CreateRateCardRequest, CreateRateCardResponse, GetRateCardRequest, GetRateCardResponse,
    ListRateCardChangesRequest, ListRateCardChangesResponse, ListRateCardsRequest,
    ListRateCardsResponse, LookupRateRequest, LookupRateResponse, RateCardUnmatched,
    SetRateCardRatesRequest, SetRateCardRatesResponse, rate_cards_service_server::RateCardsService,
};
use meteroid_grpc_mapping::activity::entries::{
    decode_cursor, encode_cursor, entries_to_proto, resolve_limit,
};
use meteroid_store::domain::entity_activity::EntityType;
use meteroid_store::domain::rate_cards::{
    RateCardDenomination, RateCardKeyRemoval, RateCardNew, RateCardRateInput, RateCardRatesSet,
    rates_at,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::RateCardsInterface;
use meteroid_store::repositories::entity_activity::EntityActivityInterface;
use rust_decimal::Decimal;
use tonic::{Request, Response, Status};

use super::{RateCardsServiceComponents, mapping};

fn store_status(report: Report<StoreError>) -> Status {
    match report.current_context() {
        StoreError::InvalidArgument(message) => Status::invalid_argument(message.clone()),
        StoreError::ValueNotFound(message) => Status::not_found(message.clone()),
        StoreError::DuplicateValue { key, .. } => Status::already_exists(format!(
            "a rate card with code '{}' already exists",
            key.clone().unwrap_or_default()
        )),
        _ => Status::internal("Error in rate cards service"),
    }
}

fn date(value: &str) -> Result<NaiveDate, Status> {
    NaiveDate::from_str(value).map_err(|_| Status::invalid_argument(format!("not a date: {value}")))
}

fn decimal(value: &str) -> Result<Decimal, Status> {
    Decimal::from_str(value.trim())
        .map_err(|_| Status::invalid_argument(format!("not a decimal: {value}")))
}

fn today() -> NaiveDate {
    common_clock::now().date_naive()
}

#[tonic::async_trait]
impl RateCardsService for RateCardsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_rate_card(
        &self,
        request: Request<CreateRateCardRequest>,
    ) -> Result<Response<CreateRateCardResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();
        let denomination = match inner.credit_unit_id.filter(|id| !id.is_empty()) {
            Some(id) => RateCardDenomination::CreditUnit(CreditUnitId::from_proto(id)?),
            None => RateCardDenomination::Money,
        };
        let unit_size = match inner.unit_size.as_deref().map(str::trim) {
            Some(size) if !size.is_empty() => decimal(size)?,
            _ => Decimal::ONE,
        };
        let on_unmatched = RateCardUnmatched::try_from(inner.on_unmatched)
            .map(mapping::unmatched_from_proto)
            .map_err(|_| Status::invalid_argument("unknown unmatched policy"))?;
        let card = self
            .store
            .create_rate_card(
                actor,
                RateCardNew {
                    tenant_id,
                    code: inner.code,
                    name: inner.name,
                    metric_id: BillableMetricId::from_proto(inner.metric_id)?,
                    dimensions: inner.dimensions,
                    denomination,
                    unit_size,
                    on_unmatched,
                },
            )
            .await
            .map_err(store_status)?;
        Ok(Response::new(CreateRateCardResponse {
            card: Some(mapping::card_to_proto(card)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_rate_cards(
        &self,
        request: Request<ListRateCardsRequest>,
    ) -> Result<Response<ListRateCardsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let metric_id = request
            .into_inner()
            .billable_metric_id
            .filter(|id| !id.is_empty())
            .map(BillableMetricId::from_proto)
            .transpose()?;
        let cards = self
            .store
            .list_rate_cards(tenant_id, metric_id)
            .await
            .map_err(store_status)?;
        Ok(Response::new(ListRateCardsResponse {
            cards: cards.into_iter().map(mapping::card_to_proto).collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_rate_card(
        &self,
        request: Request<GetRateCardRequest>,
    ) -> Result<Response<GetRateCardResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let at = inner
            .at
            .as_deref()
            .filter(|day| !day.trim().is_empty())
            .map(date)
            .transpose()?
            .unwrap_or_else(today);
        let (card, entries) = self
            .store
            .get_rate_card(tenant_id, RateCardId::from_proto(inner.id)?)
            .await
            .map_err(store_status)?;
        Ok(Response::new(GetRateCardResponse {
            card: Some(mapping::card_to_proto(card)),
            rates: rates_at(&entries, at)
                .into_iter()
                .map(mapping::entry_to_proto)
                .collect(),
            entries: entries.iter().map(mapping::entry_to_proto).collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_rate_card_rates(
        &self,
        request: Request<SetRateCardRatesRequest>,
    ) -> Result<Response<SetRateCardRatesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();
        let rates = inner
            .rates
            .into_iter()
            .map(|rate| {
                Ok(RateCardRateInput {
                    key: rate.key,
                    currency: rate.currency,
                    rate: decimal(&rate.rate)?,
                    label: rate.label,
                })
            })
            .collect::<Result<Vec<_>, Status>>()?;
        let set = RateCardRatesSet {
            effective_from: inner
                .effective_from
                .as_deref()
                .filter(|day| !day.trim().is_empty())
                .map(date)
                .transpose()?,
            rates,
            remove: inner
                .remove
                .into_iter()
                .map(|removal| RateCardKeyRemoval {
                    key: removal.key,
                    currency: removal.currency,
                })
                .collect(),
        };
        let summary = self
            .store
            .set_rate_card_rates(
                actor,
                tenant_id,
                RateCardId::from_proto(inner.rate_card_id)?,
                set,
                today(),
            )
            .await
            .map_err(store_status)?;
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(Response::new(SetRateCardRatesResponse {
            inserted: count(summary.inserted),
            unchanged: count(summary.unchanged),
            replaced: count(summary.replaced),
            removed: count(summary.removed),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn lookup_rate(
        &self,
        request: Request<LookupRateRequest>,
    ) -> Result<Response<LookupRateResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let on = inner
            .on
            .as_deref()
            .map(date)
            .transpose()?
            .unwrap_or_else(today);
        let (card, entries) = self
            .store
            .get_rate_card(tenant_id, RateCardId::from_proto(inner.rate_card_id)?)
            .await
            .map_err(store_status)?;
        let dimensions: BTreeMap<String, String> = inner.dimensions.into_iter().collect();
        let currency = inner
            .currency
            .as_deref()
            .filter(|code| !code.trim().is_empty());
        let found = card.entry_for(&entries, &dimensions, on, currency);
        Ok(Response::new(LookupRateResponse {
            entry: found.map(mapping::entry_to_proto),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_rate_card_changes(
        &self,
        request: Request<ListRateCardChangesRequest>,
    ) -> Result<Response<ListRateCardChangesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let org_id = request.organization()?;
        let inner = request.into_inner();
        let id = RateCardId::from_proto(inner.rate_card_id)?;
        let before = inner
            .cursor
            .as_deref()
            .filter(|cursor| !cursor.is_empty())
            .map(decode_cursor)
            .transpose()?;
        let page = self
            .store
            .list_entity_activities(
                tenant_id,
                &EntityType::RateCard.to_string(),
                id.as_uuid(),
                before,
                resolve_limit(inner.limit),
            )
            .await
            .map_err(store_status)?;
        let entries = entries_to_proto(&self.store, tenant_id, org_id, page.items).await?;
        Ok(Response::new(ListRateCardChangesResponse {
            changes: entries
                .into_iter()
                .filter_map(mapping::change_to_proto)
                .collect(),
            next_cursor: page.next_cursor.map(encode_cursor),
        }))
    }
}
