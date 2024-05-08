/*


Kept for reference until the EventBus is implemented in the service, and the Slot transaction is implemented


 */

// use cornucopia_async::{GenericClient, Params};
// use time::Time;
// use tonic::{Request, Response, Status};
// use uuid::Uuid;
//
// use common_grpc::middleware::server::auth::RequestExt;
// use meteroid_grpc::meteroid::api::components::v1::fee::r#type::Fee;
// use meteroid_grpc::meteroid::api::subscriptions::v1::cancel_subscription_request::EffectiveAt;
// use meteroid_grpc::meteroid::api::subscriptions::v1::subscriptions_service_server::SubscriptionsService;
// use meteroid_grpc::meteroid::api::subscriptions::v1::ApplySlotsDeltaResponse;
// use meteroid_grpc::meteroid::api::subscriptions::v1::{
//     ApplySlotsDeltaRequest, CancelSubscriptionRequest, CancelSubscriptionResponse,
// };
// use meteroid_grpc::meteroid::api::subscriptions::v1::{
//     CreateSubscriptionRequest, CreateSubscriptionResponse, GetSubscriptionDetailsRequest,
//     GetSubscriptionDetailsResponse, ListSubscriptionsRequest, ListSubscriptionsResponse,
// };
// use meteroid_repository as db;
// use meteroid_repository::BillingPeriodEnum;
// use meteroid_store::repositories::SubscriptionInterface;
//
// use crate::api::shared;
// use crate::api::subscriptions::error::SubscriptionApiError;
// use crate::api::subscriptions::{mapping, SubscriptionServiceComponents};
// use crate::api::utils::{parse_uuid, uuid_gen, PaginationExt};
// // use crate::compute::clients::subscription::SubscriptionClient;
// // use crate::compute::fees::shared::CadenceExtractor;
// // use crate::compute::fees::ComputeInvoiceLine;
// use crate::compute::period;
// use common_eventbus::Event;
// use crate::mapping::common::{chrono_to_date, chrono_to_datetime};
// use crate::models::InvoiceLine;
// use crate::{parse_uuid, parse_uuid_opt};
//
// use super::super::pricecomponents;
//
// #[tonic::async_trait]
// impl SubscriptionsService for SubscriptionServiceComponents {
//     #[tracing::instrument(skip_all)]
//     async fn list_subscriptions(
//         &self,
//         request: Request<ListSubscriptionsRequest>,
//     ) -> Result<Response<ListSubscriptionsResponse>, Status> {
//         let tenant_id = request.tenant()?;
//         let inner = request.into_inner();
//
//         let res = self.store.list_subscriptions(
//             tenant_id,
//             parse_uuid_opt!(inner.customer_id),
//             parse_uuid_opt!(inner.plan_id),
//             inner.pagination.into(),
//         ).await.map_err(|e| {
//             SubscriptionApiError::StoreError("failed to list subscriptions".to_string(), e)
//         })?;
//
//
//         let subscriptions = res.items
//             .into_iter()
//             .map(|c| mapping::subscriptions::domain_to_proto(c).unwrap())
//             .collect();
//
//         Ok(Response::new(ListSubscriptionsResponse {
//             subscriptions,
//             pagination_meta: inner.pagination.into_response(res.total as u32),
//         }))
//     }
//
//     #[tracing::instrument(skip_all)]
//     async fn create_subscription(
//         &self,
//         request: Request<CreateSubscriptionRequest>,
//     ) -> Result<Response<CreateSubscriptionResponse>, Status> {
//         let actor = request.actor()?;
//         let tenant_id = request.tenant()?;
//         let inner = request.into_inner();
//         let mut connection = self.get_connection().await?;
//         let transaction = self.get_transaction(&mut connection).await?;
//         let plan_version_id = parse_uuid!(&inner.plan_version_id)?;
//
//         let plan_version = db::plans::get_plan_version_by_id()
//             .bind(&transaction, &plan_version_id, &tenant_id)
//             .one()
//             .await
//             .map_err(|e| {
//                 SubscriptionApiError::DatabaseError("failed to get plan version".to_string(), e)
//             })?;
//
//         if plan_version.is_draft_version {
//             return Err(SubscriptionApiError::InvalidArgument(
//                 "cannot create subscription for a draft version".to_string(),
//             )
//                 .into());
//         }
//
//         // validate that for each plan_parameter that we have a valid input_parameter value for it.
//         let components = pricecomponents::ext::list_price_components(
//             plan_version_id.clone(),
//             tenant_id.clone(),
//             &transaction,
//         )
//             .await?;
//         let plan_parameters = pricecomponents::ext::components_to_params(components);
//
//         let mut slot_transactions = Vec::new();
//
//         if !plan_parameters.is_empty() {
//             if let Some(input_parameters) = &inner.parameters {
//                 for plan_parameter in plan_parameters {
//                     match plan_parameter {
//                         pricecomponents::ext::PlanParameter::BillingPeriodTerm => {
//                             if input_parameters.committed_billing_period.is_none() {
//                                 return Err(SubscriptionApiError::MissingArgument(
//                                     "Billing Period parameter".to_string(),
//                                 )
//                                     .into());
//                             }
//                         }
//                         pricecomponents::ext::PlanParameter::CapacityThresholdValue {
//                             component_id,
//                             capacity_values,
//                         } => {
//                             input_parameters
//                                 .parameters
//                                 .iter()
//                                 .find(|p| {
//                                     p.component_id == component_id
//                                         && capacity_values.contains(&p.value)
//                                 })
//                                 .ok_or(SubscriptionApiError::MissingArgument(format!(
//                                     "missing or invalid capacity threshold parameter for component {}",
//                                     component_id
//                                 )))?;
//                         }
//                         pricecomponents::ext::PlanParameter::CommittedSlot { component_id } => {
//                             let (price_component_id, slots) = input_parameters
//                                 .parameters
//                                 .iter()
//                                 .find_map(|p| {
//                                     if p.component_id == component_id && p.value > 0 {
//                                         Some((parse_uuid!(&p.component_id).ok()?, p.value))
//                                     } else {
//                                         None
//                                     }
//                                 })
//                                 .ok_or(SubscriptionApiError::MissingArgument(format!(
//                                     "committed_slot parameter for component {}",
//                                     component_id
//                                 )))?;
//
//                             slot_transactions.push((price_component_id, slots));
//                         }
//                     }
//                 }
//             } else {
//                 return Err(SubscriptionApiError::MissingArgument("parameters".to_string()).into());
//             }
//         }
//
//         let serialized_params = inner
//             .parameters
//             .map(|parameters| {
//                 serde_json::to_value(&parameters).map_err(|e| {
//                     SubscriptionApiError::SerializationError(
//                         "Unable to serialize parameters".to_string(),
//                         e,
//                     )
//                 })
//             })
//             .transpose()?;
//
//         let billing_start = inner
//             .billing_start
//             .map(|d| {
//                 shared::mapping::date::from_proto(d).map_err(|e| {
//                     SubscriptionApiError::InvalidArgument(format!("unable to convert date - {}", e))
//                 })
//             })
//             .transpose()?
//             .ok_or(SubscriptionApiError::MissingArgument(
//                 "billing_start".to_string(),
//             ))?;
//
//         let billing_start_midnight = billing_start.with_time(Time::MIDNIGHT);
//
//         let params = db::subscriptions::CreateSubscriptionParams {
//             id: uuid_gen::v7(),
//             tenant_id: tenant_id.clone(),
//             created_by: actor,
//             plan_version_id: plan_version_id.clone(),
//             parameters: serialized_params,
//             // TODO can optimize, but with the component-based approach that's ok as long as it is the minimum
//             effective_billing_period: BillingPeriodEnum::MONTHLY,
//             billing_day: inner.billing_day as i16,
//             customer_id: parse_uuid!(&inner.customer_id)?,
//             billing_end: inner
//                 .billing_end
//                 .map(|d| {
//                     shared::mapping::date::from_proto(d).map_err(|e| {
//                         SubscriptionApiError::InvalidArgument(format!(
//                             "unable to convert date - {}",
//                             e
//                         ))
//                     })
//                 })
//                 .transpose()?,
//             billing_start,
//             net_terms: inner.net_terms,
//         };
//
//         let subscription_id = db::subscriptions::create_subscription()
//             .params(&transaction, &params)
//             .one()
//             .await
//             .map_err(|e| {
//                 SubscriptionApiError::DatabaseError("failed to create subscription".to_string(), e)
//             })?;
//
//         for (price_component_id, slots) in slot_transactions {
//             create_slot_transaction(
//                 &transaction,
//                 subscription_id,
//                 price_component_id,
//                 0, // no prev slots yet as it is first transaction
//                 slots as i32,
//                 billing_start_midnight.clone(),
//                 billing_start_midnight.clone(),
//             )
//                 .await?;
//         }
//
//         let subscription = self
//             .store
//             .get_subscription_details(subscription_id.clone())
//             .await
//             .map_err(|e| {
//                 SubscriptionApiError::StoreError(
//                     "failed to retrieve subscription details".to_string(),
//                     e,
//                 )
//             })?;
//
//         let invoice_lines = self
//             .compute_service
//             .compute_dated_invoice_lines(&subscription.billing_start_date, subscription.clone()) // TODO avoid cloning
//             .await
//             .map_err(|e| {
//                 SubscriptionApiError::CalculationError(
//                     "failed to calculate invoice lines".to_string(),
//                     e,
//                 )
//             })?;
//
//         let total = invoice_lines.iter().map(|line| line.total).sum();
//
//         let serialized_invoice_lines = serde_json::to_value(invoice_lines).map_err(|e| {
//             SubscriptionApiError::SerializationError(
//                 "failed to serialize invoice lines".to_string(),
//                 e,
//             )
//         })?;
//
//         let params = db::invoices::CreateInvoiceParams {
//             id: common_utils::uuid::v7(),
//             invoicing_provider: db::InvoicingProviderEnum::STRIPE,
//             status: db::InvoiceStatusEnum::FINALIZED,
//             invoice_date: chrono_to_date(subscription.billing_start_date)?,
//             plan_version_id,
//             tenant_id: subscription.tenant_id,
//             customer_id: subscription.customer_id,
//             subscription_id: subscription.id,
//             currency: subscription.currency.clone(),
//             days_until_due: subscription.net_terms,
//             line_items: serialized_invoice_lines,
//             amount_cents: Some(total),
//         };
//
//         db::invoices::create_invoice()
//             .params(&transaction, &params)
//             .one()
//             .await
//             .map_err(|e| {
//                 log::error!("Failed to create invoice: {:?}", e);
//                 SubscriptionApiError::DatabaseError("failed to create invoice".to_string(), e)
//             })?;
//
//         // TODO drop when switch to store
//         let subscription = subscription_by_id(&transaction, &subscription_id, &tenant_id).await?;
//
//         transaction.commit().await.map_err(|e| {
//             SubscriptionApiError::DatabaseError("failed to commit transaction".to_string(), e)
//         })?;
//
//         let _ = self
//             .eventbus
//             .publish(Event::subscription_created(
//                 actor,
//                 subscription.id,
//                 subscription.tenant_id,
//             ))
//             .await;
//
//         let _ = self
//             .eventbus
//             .publish(Event::invoice_finalized(params.id, params.tenant_id))
//             .await;
//
//         let rs = mapping::subscriptions::db_to_proto(subscription)?;
//
//         Ok(Response::new(CreateSubscriptionResponse {
//             subscription: Some(rs),
//         }))
//     }
//
//     #[tracing::instrument(skip_all)]
//     async fn get_subscription_details(
//         &self,
//         request: Request<GetSubscriptionDetailsRequest>,
//     ) -> Result<Response<GetSubscriptionDetailsResponse>, Status> {
//         let tenant_id = request.tenant()?;
//         let inner = request.into_inner();
//         let connection = self.get_connection().await?;
//
//         let subscription = subscription_by_id(
//             &connection,
//             &parse_uuid!(inner.subscription_id)?,
//             &tenant_id,
//         )
//             .await?;
//
//         let rs = mapping::subscriptions::db_to_proto(subscription)?;
//
//         Ok(Response::new(GetSubscriptionDetailsResponse {
//             subscription: Some(rs),
//         }))
//     }
//
//     #[tracing::instrument(skip_all)]
//     async fn apply_slots_delta(
//         &self,
//         request: Request<ApplySlotsDeltaRequest>,
//     ) -> Result<Response<ApplySlotsDeltaResponse>, Status> {
//         let tenant_id = request.tenant()?;
//
//         let inner = request.into_inner();
//
//         if inner.delta == 0 {
//             return Err(
//                 SubscriptionApiError::InvalidArgument("delta should not be 0".to_string()).into(),
//             );
//         }
//
//         let subscription_id = parse_uuid!(inner.subscription_id)?;
//         let price_component_id = parse_uuid!(inner.price_component_id)?;
//
//         let mut connection = self.get_connection().await?;
//         let transaction = self.get_transaction(&mut connection).await?;
//
//         let now_chrono = chrono::Utc::now().naive_utc();
//         let now = chrono_to_datetime(now_chrono)?;
//
//         let subscription = &SubscriptionClient::fetch_subscription_details(
//             &transaction,
//             &subscription_id,
//             &tenant_id,
//             &now_chrono.date(),
//         )
//             .await
//             .map_err(|e| {
//                 SubscriptionApiError::SubscriptionDetailsError(
//                     "failed to fetch subscription details".to_string(),
//                     e,
//                 )
//             })?;
//
//         let price_component = subscription
//             .price_components
//             .iter()
//             .find(|c| c.id == price_component_id.to_string())
//             .ok_or_else(|| {
//                 SubscriptionApiError::InvalidArgument(format!(
//                     "Price component {} not found",
//                     price_component_id
//                 ))
//             })?;
//
//         let component_fee = price_component
//             .fee
//             .fee
//             .as_ref()
//             .and_then(|c| match c {
//                 Fee::SlotBased(slot) => Some(slot),
//                 _ => None,
//             })
//             .ok_or_else(|| {
//                 SubscriptionApiError::InvalidArgument(format!(
//                     "Price component {} does not contain slot based fee",
//                     price_component_id
//                 ))
//             })?;
//
//         lock_subscription_for_update(&transaction, &subscription_id).await?;
//
//         let active_slots =
//             get_active_slots(&transaction, &subscription_id, &price_component_id, &now).await?;
//
//         let mut finalized_invoice = None;
//
//         if inner.delta < 0 {
//             let billing_period = component_fee
//                 .pricing
//                 .as_ref()
//                 .and_then(|x| x.pricing.as_ref())
//                 .ok_or_else(|| {
//                     SubscriptionApiError::InvalidArgument(format!(
//                         "Missing pricing details for price component {}",
//                         price_component_id
//                     ))
//                 })?
//                 .extract_cadence(subscription)
//                 .map_err(|e| {
//                     SubscriptionApiError::SubscriptionDetailsError(
//                         "failed to extract cadence from pricing".to_string(),
//                         e,
//                     )
//                 })?;
//
//             let period_idx = period::calculate_period_idx(
//                 subscription.billing_start_date,
//                 subscription.billing_day as u32,
//                 now_chrono.date(),
//                 billing_period,
//             );
//
//             let (_, period_end) = period::calculate_period_range(
//                 subscription.billing_start_date,
//                 subscription.billing_day as u32,
//                 period_idx,
//                 billing_period,
//             );
//
//             let effective_at = chrono_to_date(period_end)?
//                 .next_day()
//                 .unwrap() // safe otherwise bug
//                 .with_time(Time::MIDNIGHT);
//
//             let active_slots_bp_end = get_active_slots(
//                 &transaction,
//                 &subscription_id,
//                 &price_component_id,
//                 &effective_at,
//             )
//                 .await?;
//
//             if (active_slots_bp_end - inner.delta.abs()) < 0 {
//                 return Err(SubscriptionApiError::InvalidArgument(
//                     "number of slots cannot be negative".to_string(),
//                 )
//                     .into());
//             }
//
//             create_slot_transaction(
//                 &transaction,
//                 subscription_id,
//                 price_component_id,
//                 active_slots,
//                 inner.delta,
//                 effective_at,
//                 now,
//             )
//                 .await?;
//         } else {
//             create_slot_transaction(
//                 &transaction,
//                 subscription_id,
//                 price_component_id,
//                 active_slots,
//                 inner.delta,
//                 now,
//                 now,
//             )
//                 .await?;
//
//             let invoice_line: InvoiceLine = component_fee
//                 .compute(subscription, price_component, Some(inner.delta as u64))
//                 .map_err(|e| {
//                     SubscriptionApiError::SubscriptionDetailsError(
//                         "failed to compute invoice line for price component".to_string(),
//                         e,
//                     )
//                 })?
//                 .ok_or_else(|| {
//                     SubscriptionApiError::InvalidArgument(
//                         "failed to compute invoice line for price component".to_string(),
//                     )
//                 })?;
//
//             let invoice_lines = vec![invoice_line];
//             let total = Some(invoice_lines.iter().map(|line| line.total).sum());
//
//             let serialized_invoice_lines = serde_json::to_value(invoice_lines).map_err(|e| {
//                 SubscriptionApiError::SerializationError(
//                     "failed to serialize invoice lines".to_string(),
//                     e,
//                 )
//             })?;
//
//             let params = db::invoices::CreateInvoiceParams {
//                 id: common_utils::uuid::v7(),
//                 invoicing_provider: db::InvoicingProviderEnum::STRIPE,
//                 status: db::InvoiceStatusEnum::FINALIZED,
//                 invoice_date: now.date(),
//                 tenant_id: subscription.tenant_id,
//                 plan_version_id: subscription.plan_version_id,
//                 customer_id: subscription.customer_id,
//                 subscription_id: subscription.id,
//                 currency: subscription.currency.clone(),
//                 days_until_due: subscription.net_terms,
//                 line_items: serialized_invoice_lines,
//                 amount_cents: total,
//             };
//
//             db::invoices::create_invoice()
//                 .params(&transaction, &params)
//                 .one()
//                 .await
//                 .map_err(|e| {
//                     SubscriptionApiError::DatabaseError("failed to create invoice".to_string(), e)
//                 })?;
//
//             finalized_invoice = Some(params.id);
//         }
//
//         transaction.commit().await.map_err(|e| {
//             SubscriptionApiError::DatabaseError("failed to commit transaction".to_string(), e)
//         })?;
//
//         if let Some(invoice_id) = finalized_invoice {
//             let _ = self
//                 .eventbus
//                 .publish(Event::invoice_finalized(invoice_id, subscription.tenant_id))
//                 .await;
//         }
//
//         Ok(Response::new(ApplySlotsDeltaResponse {
//             // fetch from db instead?
//             active_slots: (active_slots + inner.delta.max(0)) as u32,
//         }))
//     }
//
//     #[tracing::instrument(skip_all)]
//     async fn cancel_subscription(
//         &self,
//         request: Request<CancelSubscriptionRequest>,
//     ) -> Result<Response<CancelSubscriptionResponse>, Status> {
//         let actor = request.actor()?;
//         let tenant_id = request.tenant()?;
//         let inner = request.into_inner();
//         let subscription_id = parse_uuid!(inner.subscription_id)?;
//
//         let mut connection = self.get_connection().await?;
//         let transaction = self.get_transaction(&mut connection).await?;
//
//         let now_chrono = chrono::Utc::now().naive_utc();
//         let now = chrono_to_datetime(now_chrono)?;
//
//         let subscription = &SubscriptionClient::fetch_subscription_details(
//             &transaction,
//             &subscription_id,
//             &tenant_id,
//             &now_chrono.date(),
//         )
//             .await
//             .map_err(|e| {
//                 SubscriptionApiError::SubscriptionDetailsError(
//                     "failed to fetch subscription details".to_string(),
//                     e,
//                 )
//             })?;
//
//         let effective_at = inner.effective_at();
//
//         let period_idx = period::calculate_period_idx(
//             subscription.billing_start_date,
//             subscription.billing_day as u32,
//             now_chrono.date(),
//             subscription.effective_billing_period,
//         );
//
//         let (_, period_end) = period::calculate_period_range(
//             subscription.billing_start_date,
//             subscription.billing_day as u32,
//             period_idx,
//             subscription.effective_billing_period,
//         );
//
//         let current_period_end = chrono_to_date(period_end)?;
//
//         let billing_end_date = match effective_at {
//             EffectiveAt::BillingPeriodEnd => current_period_end,
//         };
//
//         db::subscriptions::cancel_subscription()
//             .bind(&transaction, &billing_end_date, &now, &subscription_id)
//             .await
//             .map_err(|e| {
//                 SubscriptionApiError::DatabaseError("failed to cancel subscription".to_string(), e)
//             })?;
//
//         transaction.commit().await.map_err(|e| {
//             SubscriptionApiError::DatabaseError("failed to commit transaction".to_string(), e)
//         })?;
//
//         let _ = self
//             .eventbus
//             .publish(Event::subscription_canceled(
//                 actor,
//                 subscription.id,
//                 subscription.tenant_id,
//             ))
//             .await;
//
//         let subscription = subscription_by_id(&connection, &subscription_id, &tenant_id).await?;
//
//         let rs = mapping::subscriptions::db_to_proto(subscription)?;
//
//         Ok(Response::new(CancelSubscriptionResponse {
//             subscription: Some(rs),
//         }))
//     }
// }
//
// async fn subscription_by_id<C: GenericClient>(
//     transaction: &C,
//     subscription_id: &Uuid,
//     tenant_id: &Uuid,
// ) -> Result<db::subscriptions::Subscription, SubscriptionApiError> {
//     db::subscriptions::get_subscription_by_id()
//         .bind(transaction, subscription_id, tenant_id)
//         .one()
//         .await
//         .map_err(|e| {
//             SubscriptionApiError::DatabaseError("failed to get subscription".to_string(), e)
//         })
// }
//
// #[tracing::instrument(skip(transaction))]
// async fn get_active_slots<C: GenericClient>(
//     transaction: &C,
//     subscription_id: &Uuid,
//     price_component_id: &Uuid,
//     timestamp: &time::PrimitiveDateTime,
// ) -> Result<i32, SubscriptionApiError> {
//     let slots = db::slot_transactions::get_active_slots()
//         .bind(transaction, subscription_id, price_component_id, timestamp)
//         .opt()
//         .await
//         .map_err(|e| {
//             SubscriptionApiError::DatabaseError("failed to get active slots".to_string(), e)
//         })?
//         .map(|x| x as i32)
//         .unwrap_or(0i32);
//
//     Ok(slots)
// }
//
// #[tracing::instrument(skip(transaction))]
// async fn create_slot_transaction<C: GenericClient>(
//     transaction: &C,
//     subscription_id: Uuid,
//     price_component_id: Uuid,
//     prev_active_slots: i32, // computed active slots excluding this transaction
//     delta: i32,
//     effective_at: time::PrimitiveDateTime,
//     transaction_at: time::PrimitiveDateTime,
// ) -> Result<Uuid, SubscriptionApiError> {
//     db::slot_transactions::create_slot_transaction()
//         .params(
//             transaction,
//             &db::slot_transactions::CreateSlotTransactionParams {
//                 id: uuid_gen::v7(),
//                 price_component_id,
//                 subscription_id,
//                 delta,
//                 prev_active_slots,
//                 effective_at,
//                 transaction_at,
//             },
//         )
//         .one()
//         .await
//         .map_err(|e| {
//             SubscriptionApiError::DatabaseError("failed to create slot transaction".to_string(), e)
//         })
// }
//
// async fn lock_subscription_for_update<C: GenericClient>(
//     transaction: &C,
//     subscription_id: &Uuid,
// ) -> Result<(), SubscriptionApiError> {
//     transaction
//         .query_one(
//             "SELECT 1 FROM subscription WHERE id = $1 FOR UPDATE",
//             &[&subscription_id],
//         )
//         .await
//         .map_err(|e| {
//             SubscriptionApiError::DatabaseError(
//                 "failed to lock subscription for update".to_string(),
//                 e,
//             )
//         })?;
//
//     Ok(())
// }
