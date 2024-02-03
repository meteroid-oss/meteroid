use super::super::pricecomponents;
use crate::api::services::shared;
use crate::api::services::subscriptions::{mapping, ErrorWrapper, SubscriptionServiceComponents};
use crate::api::services::utils::{parse_uuid, uuid_gen, PaginationExt};
use crate::compute::clients::subscription::SubscriptionClient;
use crate::compute::fees::shared::CadenceExtractor;
use crate::compute::fees::ComputeInvoiceLine;
use crate::compute::period;
use crate::mapping::common::{chrono_to_date, chrono_to_datetime, date_to_chrono};
use crate::models::InvoiceLine;
use crate::parse_uuid;
use common_grpc::middleware::server::auth::RequestExt;
use cornucopia_async::{GenericClient, Params};
use meteroid_grpc::meteroid::api::components::v1::fee::r#type::Fee;
use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsService;
use meteroid_grpc::meteroid::api::subscriptions::v1::ApplySlotsDeltaRequest;
use meteroid_grpc::meteroid::api::subscriptions::v1::ApplySlotsDeltaResponse;
use meteroid_grpc::meteroid::api::subscriptions::v1::{
    CreateSubscriptionRequest, CreateSubscriptionResponse, GetSubscriptionDetailsRequest,
    GetSubscriptionDetailsResponse, ListSubscriptionsPerPlanRequest,
    ListSubscriptionsPerPlanResponse,
};
use meteroid_repository as db;
use meteroid_repository::BillingPeriodEnum;
use std::sync::Arc;
use time::Time;
use tonic::{Request, Response, Status};
use tracing::error;
use uuid::Uuid;

#[tonic::async_trait]
impl SubscriptionsService for SubscriptionServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_subscriptions_per_plan(
        &self,
        request: Request<ListSubscriptionsPerPlanRequest>,
    ) -> Result<Response<ListSubscriptionsPerPlanResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let connection = self.get_connection().await?;

        let params = db::subscriptions::ListSubscriptionsPerPlanParams {
            plan_id: parse_uuid!(inner.plan_id)?,
            tenant_id,
            limit: inner.pagination.limit(),
            offset: inner.pagination.offset(),
        };

        let subscriptions = db::subscriptions::list_subscriptions_per_plan()
            .params(&connection, &params)
            .all()
            .await
            .map_err(|e| {
                Status::internal("Failed to list subscriptions")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let total = subscriptions.first().map(|c| c.total_count).unwrap_or(0);

        let subscriptions = subscriptions
            .into_iter()
            .map(|c| mapping::subscriptions::list_db_to_proto(c).unwrap())
            .collect();

        Ok(Response::new(ListSubscriptionsPerPlanResponse {
            subscriptions,
            pagination_meta: inner.pagination.into_response(total as u32),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_subscription(
        &self,
        request: Request<CreateSubscriptionRequest>,
    ) -> Result<Response<CreateSubscriptionResponse>, Status> {
        let actor = request.actor()?;
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let mut connection = self.get_connection().await?;
        let transaction = self.get_transaction(&mut connection).await?;
        let plan_version_id = parse_uuid!(&inner.plan_version_id)?;

        let plan_version = db::plans::get_plan_version_by_id()
            .bind(&transaction, &plan_version_id, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to get plan version")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        if plan_version.is_draft_version {
            return Err(Status::invalid_argument(
                "Cannot create subscription for a draft version".to_string(),
            ));
        }

        // validate that for each plan_parameter that we have a valid input_parameter value for it.
        let components = pricecomponents::ext::list_price_components(
            plan_version_id.clone(),
            tenant_id.clone(),
            &transaction,
        )
        .await?;
        let plan_parameters = pricecomponents::ext::components_to_params(components);

        let mut slot_transactions = Vec::new();

        if !plan_parameters.is_empty() {
            if let Some(input_parameters) = &inner.parameters {
                for plan_parameter in plan_parameters {
                    match plan_parameter {
                        pricecomponents::ext::PlanParameter::BillingPeriodTerm => {
                            if input_parameters.committed_billing_period.is_none() {
                                return Err(Status::invalid_argument(
                                    "Missing Billing Period parameter".to_string(),
                                ));
                            }
                        }
                        pricecomponents::ext::PlanParameter::CapacityThresholdValue {
                            component_id,
                            capacity_values,
                        } => {
                            input_parameters
                                .parameters
                                .iter()
                                .find(|p| {
                                    p.component_id == component_id
                                        && capacity_values.contains(&p.value)
                                })
                                .ok_or(Status::invalid_argument(format!(
                                    "Missing or invalid capacity threshold parameter for component {}",
                                    component_id
                                )))?;
                        }
                        pricecomponents::ext::PlanParameter::CommittedSlot { component_id } => {
                            let (price_component_id, slots) = input_parameters
                                .parameters
                                .iter()
                                .find_map(|p| {
                                    if p.component_id == component_id && p.value > 0 {
                                        Some((parse_uuid!(&p.component_id).ok()?, p.value))
                                    } else {
                                        None
                                    }
                                })
                                .ok_or(Status::invalid_argument(format!(
                                    "Missing committed_slot parameter for component {}",
                                    component_id
                                )))?;

                            slot_transactions.push((price_component_id, slots));
                        }
                    }
                }
            } else {
                return Err(Status::invalid_argument("Missing parameters".to_string()));
            }
        }

        let serialized_params = inner
            .parameters
            .map(|parameters| {
                serde_json::to_value(&parameters).map_err(|e| {
                    Status::invalid_argument("Unable to serialize parameters")
                        .set_source(Arc::new(e))
                        .clone()
                })
            })
            .transpose()?;

        let billing_start = inner
            .billing_start
            .map(|d| {
                shared::mapping::date::from_proto(d).map_err(|e| {
                    Status::internal("Unable to convert date")
                        .set_source(Arc::new(e))
                        .clone()
                })
            })
            .transpose()?
            .ok_or(Status::invalid_argument("Missing billing_start"))?;

        let billing_start_midnight = billing_start.with_time(Time::MIDNIGHT);

        let params = db::subscriptions::CreateSubscriptionParams {
            id: uuid_gen::v7(),
            tenant_id: tenant_id.clone(),
            created_by: actor,
            plan_version_id: plan_version_id.clone(),
            parameters: serialized_params,
            status: db::SubscriptionStatusEnum::PENDING,
            // TODO can optimize, but with the component-based approach that's ok as long as it is the minimum
            effective_billing_period: BillingPeriodEnum::MONTHLY,
            billing_day: inner.billing_day as i16,
            customer_id: parse_uuid!(&inner.customer_id)?,
            billing_end: inner
                .billing_end
                .map(|d| {
                    shared::mapping::date::from_proto(d).map_err(|e| {
                        Status::internal("Unable to convert date")
                            .set_source(Arc::new(e))
                            .clone()
                    })
                })
                .transpose()?,
            billing_start,
            net_terms: inner.net_terms,
        };

        let subscription_id = db::subscriptions::create_subscription()
            .params(&transaction, &params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to create subscription")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        for (price_component_id, slots) in slot_transactions {
            create_slot_transaction(
                &transaction,
                subscription_id,
                price_component_id,
                0, // no prev slots yet as it is first transaction
                slots as i32,
                billing_start_midnight.clone(),
                billing_start_midnight.clone(),
            )
            .await?;
        }

        let subscription = db::subscriptions::subscription_by_id()
            .bind(&transaction, &subscription_id, &tenant_id)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to get subscription")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let invoice_lines = self
            .compute_service
            .calculate_invoice_lines(
                &transaction,
                &subscription_id,
                &date_to_chrono(subscription.billing_start_date)?,
            )
            .await
            .map_err(|e| {
                error!("Failed to calculate invoice lines: {:?}", e);
                Status::internal("Failed to calculate invoice lines")
                    .set_source(Arc::new(ErrorWrapper::from(e)))
                    .clone()
            })?;

        let serialized_invoice_lines = serde_json::to_value(invoice_lines).map_err(|e| {
            Status::internal("Failed to serialize invoice lines")
                .set_source(Arc::new(e))
                .clone()
        })?;

        let params = db::invoices::CreateInvoiceParams {
            id: common_utils::uuid::v7(),
            invoicing_provider: db::InvoicingProviderEnum::STRIPE,
            status: db::InvoiceStatusEnum::FINALIZED,
            invoice_date: subscription.billing_start_date,
            tenant_id: subscription.tenant_id,
            customer_id: subscription.customer_id,
            subscription_id: subscription.subscription_id,
            currency: subscription.currency.clone(),
            days_until_due: subscription.net_terms,
            line_items: serialized_invoice_lines,
        };

        db::invoices::create_invoice()
            .params(&transaction, &params)
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to create invoice")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        transaction.commit().await.map_err(|e| {
            Status::internal("Failed to commit transaction")
                .set_source(Arc::new(e))
                .clone()
        })?;

        let rs = mapping::subscriptions::db_to_proto(subscription)?;

        Ok(Response::new(CreateSubscriptionResponse {
            subscription: Some(rs),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_subscription_details(
        &self,
        request: Request<GetSubscriptionDetailsRequest>,
    ) -> Result<Response<GetSubscriptionDetailsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let connection = self.get_connection().await?;

        let subscription = db::subscriptions::subscription_by_id()
            .bind(
                &connection,
                &parse_uuid!(inner.subscription_id)?,
                &tenant_id,
            )
            .one()
            .await
            .map_err(|e| {
                Status::internal("Failed to get subscription")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

        let rs = mapping::subscriptions::db_to_proto(subscription)?;

        Ok(Response::new(GetSubscriptionDetailsResponse {
            subscription: Some(rs),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn apply_slots_delta(
        &self,
        request: Request<ApplySlotsDeltaRequest>,
    ) -> Result<Response<ApplySlotsDeltaResponse>, Status> {
        let inner = request.into_inner();

        if inner.delta == 0 {
            return Err(Status::invalid_argument("Delta should not be 0"));
        }

        let subscription_id = parse_uuid!(inner.subscription_id)?;
        let price_component_id = parse_uuid!(inner.price_component_id)?;

        let mut connection = self.get_connection().await?;
        let transaction = self.get_transaction(&mut connection).await?;

        let now_chrono = chrono::Utc::now().naive_utc();
        let now = chrono_to_datetime(now_chrono)?;

        let subscription = &SubscriptionClient::fetch_subscription_details(
            &transaction,
            &subscription_id,
            &now_chrono.date(),
        )
        .await
        .map_err(|e| {
            Status::internal("Failed to fetch subscription details")
                .set_source(Arc::new(ErrorWrapper::from(e)))
                .clone()
        })?;

        let price_component = subscription
            .price_components
            .iter()
            .find(|c| c.id == price_component_id.to_string())
            .ok_or_else(|| {
                Status::invalid_argument(format!(
                    "Price component {} not found",
                    price_component_id
                ))
            })?;

        let component_fee = price_component
            .fee
            .fee
            .as_ref()
            .and_then(|c| match c {
                Fee::SlotBased(slot) => Some(slot),
                _ => None,
            })
            .ok_or_else(|| {
                Status::invalid_argument(format!(
                    "Price component {} does not contain slot based fee",
                    price_component_id
                ))
            })?;

        lock_subscription_for_update(&transaction, &subscription_id).await?;

        let active_slots =
            get_active_slots(&transaction, &subscription_id, &price_component_id, &now).await?;

        if inner.delta < 0 {
            let billing_period = component_fee
                .pricing
                .as_ref()
                .and_then(|x| x.pricing.as_ref())
                .ok_or_else(|| {
                    Status::invalid_argument(format!(
                        "Missing pricing details for price component {}",
                        price_component_id
                    ))
                })?
                .extract_cadence(subscription)
                .map_err(|e| {
                    Status::internal("Failed to extract cadence from pricing")
                        .set_source(Arc::new(ErrorWrapper::from(e)))
                        .clone()
                })?;

            let period_idx = period::calculate_period_idx(
                subscription.billing_start_date,
                subscription.billing_day as u32,
                now_chrono.date(),
                billing_period,
            );

            let (_, period_end) = period::calculate_period_range(
                subscription.billing_start_date,
                subscription.billing_day as u32,
                period_idx,
                billing_period,
            );

            let effective_at = chrono_to_date(period_end)?
                .next_day()
                .unwrap() // safe otherwise bug
                .with_time(Time::MIDNIGHT);

            let active_slots_bp_end = get_active_slots(
                &transaction,
                &subscription_id,
                &price_component_id,
                &effective_at,
            )
            .await?;

            if (active_slots_bp_end - inner.delta.abs()) < 0 {
                return Err(Status::invalid_argument(
                    "number of slots cannot be negative",
                ));
            }

            create_slot_transaction(
                &transaction,
                subscription_id,
                price_component_id,
                active_slots,
                inner.delta,
                effective_at,
                now,
            )
            .await?;
        } else {
            create_slot_transaction(
                &transaction,
                subscription_id,
                price_component_id,
                active_slots,
                inner.delta,
                now,
                now,
            )
            .await?;

            let invoice_line: InvoiceLine = component_fee
                .compute(subscription, price_component, Some(inner.delta as u64))
                .map_err(|e| {
                    Status::internal("Failed to compute invoice line for price component")
                        .set_source(Arc::new(ErrorWrapper::from(e)))
                        .clone()
                })?
                .ok_or_else(|| {
                    Status::invalid_argument("Failed to compute invoice line for price component")
                })?;

            let invoice_lines = vec![invoice_line];

            let serialized_invoice_lines = serde_json::to_value(invoice_lines).map_err(|e| {
                Status::internal("Failed to serialize invoice lines")
                    .set_source(Arc::new(e))
                    .clone()
            })?;

            let params = db::invoices::CreateInvoiceParams {
                id: common_utils::uuid::v7(),
                invoicing_provider: db::InvoicingProviderEnum::STRIPE,
                status: db::InvoiceStatusEnum::FINALIZED,
                invoice_date: now.date(),
                tenant_id: subscription.tenant_id,
                customer_id: subscription.customer_id,
                subscription_id: subscription.id,
                currency: subscription.currency.clone(),
                days_until_due: subscription.net_terms,
                line_items: serialized_invoice_lines,
            };

            db::invoices::create_invoice()
                .params(&transaction, &params)
                .one()
                .await
                .map_err(|e| {
                    Status::internal("Failed to create invoice")
                        .set_source(Arc::new(e))
                        .clone()
                })?;
        }

        transaction.commit().await.map_err(|e| {
            Status::internal("Failed to commit transaction")
                .set_source(Arc::new(e))
                .clone()
        })?;

        Ok(Response::new(ApplySlotsDeltaResponse {
            // fetch from db instead?
            active_slots: (active_slots + inner.delta.max(0)) as u32,
        }))
    }
}

#[tracing::instrument(skip(transaction))]
async fn get_active_slots<C: GenericClient>(
    transaction: &C,
    subscription_id: &Uuid,
    price_component_id: &Uuid,
    timestamp: &time::PrimitiveDateTime,
) -> Result<i32, Status> {
    let slots = db::slot_transactions::get_active_slots()
        .bind(transaction, subscription_id, price_component_id, timestamp)
        .opt()
        .await
        .map_err(|e| {
            log::error!("Failed to get active slots: {:?}", e);
            Status::internal("Failed to get active slots")
                .set_source(Arc::new(e))
                .clone()
        })?
        .map(|x| x as i32)
        .unwrap_or(0i32);

    Ok(slots)
}

#[tracing::instrument(skip(transaction))]
async fn create_slot_transaction<C: GenericClient>(
    transaction: &C,
    subscription_id: Uuid,
    price_component_id: Uuid,
    prev_active_slots: i32, // computed active slots excluding this transaction
    delta: i32,
    effective_at: time::PrimitiveDateTime,
    transaction_at: time::PrimitiveDateTime,
) -> Result<Uuid, Status> {
    db::slot_transactions::create_slot_transaction()
        .params(
            transaction,
            &db::slot_transactions::CreateSlotTransactionParams {
                id: uuid_gen::v7(),
                price_component_id,
                subscription_id,
                delta,
                prev_active_slots,
                effective_at,
                transaction_at,
            },
        )
        .one()
        .await
        .map_err(|e| {
            log::error!("Failed to create slot transaction: {:?}", e);
            Status::internal("Failed to create slot transaction")
                .set_source(Arc::new(e))
                .clone()
        })
}

async fn lock_subscription_for_update<C: GenericClient>(
    transaction: &C,
    subscription_id: &Uuid,
) -> Result<(), Status> {
    transaction
        .query_one(
            "SELECT 1 FROM subscription WHERE id = $1 FOR UPDATE",
            &[&subscription_id],
        )
        .await
        .map_err(|e| {
            log::error!("Failed to lock subscription for update: {:?}", e);
            Status::internal("Failed to lock subscription for update")
                .set_source(Arc::new(e))
                .clone()
        })?;

    Ok(())
}
