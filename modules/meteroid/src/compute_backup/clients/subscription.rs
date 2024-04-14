// use std::collections::HashMap;
//
// use chrono::NaiveDate;
// use cornucopia_async::GenericClient;
// use tonic::Status;
//
// use uuid::Uuid;
//
// use crate::compute::period::calculate_period_idx;
// use crate::compute::PriceComponent;
// use crate::compute::SubscriptionDetails;
// use crate::mapping::common::date_to_chrono;
// use meteroid_grpc::meteroid::api::components::v1::fee;
// use meteroid_grpc::meteroid::api::shared::v1::BillingPeriod;
// use meteroid_grpc::meteroid::api::subscriptions::v1::SubscriptionParameters;
// use meteroid_repository::price_components::PriceComponentWithMetric;
// use meteroid_repository::BillingPeriodEnum;
//
// #[derive(Clone, Debug)]
// pub struct SubscriptionClient;
//
// // no trait here because of the generic causing issues. TODO
// impl SubscriptionClient {
//     // Fetches the price point details for a specific price point ID
//     pub async fn fetch_subscription_details<C: GenericClient>(
//         db_client: &C,
//         subscription_id: &Uuid,
//         tenant_id: &Uuid,
//         invoice_date: &NaiveDate,
//     ) -> anyhow::Result<SubscriptionDetails> {
//         let subscription = meteroid_repository::subscriptions::get_subscription_by_id()
//             .bind(db_client, subscription_id, tenant_id)
//             .one()
//             .await?;
//
//         let schedules = meteroid_repository::schedules::list_schedules_by_subscription()
//             .bind(db_client, &subscription.id)
//             .all()
//             .await?;
//
//         let price_components: Vec<PriceComponentWithMetric> =
//             meteroid_repository::price_components::list_price_components_by_subscription()
//                 .bind(db_client, &subscription.id)
//                 .all()
//                 .await?
//                 .into_iter()
//                 .collect();
//
//         let metric_ids: Vec<uuid::Uuid> = price_components
//             .iter()
//             .filter_map(|pc| pc.billable_metric_id)
//             .collect();
//
//         let metrics = meteroid_repository::billable_metrics::get_billable_metric_by_ids()
//             .bind(db_client, &metric_ids, &subscription.tenant_id)
//             .all()
//             .await?;
//
//         let metrics_map: HashMap<Uuid, meteroid_repository::billable_metrics::BillableMetric> =
//             metrics.into_iter().map(|m| (m.id, m)).collect();
//
//         let resolved_price_components: Vec<PriceComponent> = price_components.into_iter()
//             .map(|pc| {
//                 let db_metric = pc.billable_metric_id.and_then(|id| metrics_map.get(&id).cloned());
//
//                 let grpc_metric = db_metric.map(crate::api::billablemetrics::mapping::metric::db_to_server);
//                 let fee: fee::Type = serde_json::from_value(pc.fee)?;
//
//                 Ok(PriceComponent {
//                     id: pc.id.to_string(),
//                     name: pc.name,
//                     fee,
//                     product_item: pc.product_item_id.zip(pc.product_item_name).map(
//                         |(id, name)| meteroid_grpc::meteroid::api::components::v1::price_component::ProductItem {
//                             id: id.to_string(),
//                             name: name.to_string(),
//                         },
//                     ),
//                     metric: grpc_metric,
//                 })
//             })
//             .collect::<anyhow::Result<Vec<_>>>()?;
//
//         // todo @gaspard implement this correctly
//         let schedule = schedules
//             .first()
//             .cloned()
//             .map(crate::api::schedules::mapping::schedules::db_to_server)
//             .transpose()?;
//
//         let parameters: SubscriptionParameters = subscription
//             .input_parameters
//             .as_ref()
//             .map(|v| serde_json::from_value(v.clone()))
//             .transpose()?
//             .unwrap_or_else(|| SubscriptionParameters {
//                 parameters: vec![],
//                 committed_billing_period: None,
//             });
//
//         let billing_start_date = date_to_chrono(subscription.billing_start_date)?;
//
//         let effective_billing_period: BillingPeriod = match subscription.effective_billing_period {
//             BillingPeriodEnum::MONTHLY => BillingPeriod::Monthly,
//             BillingPeriodEnum::QUARTERLY => BillingPeriod::Quarterly,
//             BillingPeriodEnum::ANNUAL => BillingPeriod::Annual,
//         };
//
//         Ok(SubscriptionDetails {
//             id: subscription.id,
//             tenant_id: subscription.tenant_id,
//             customer_id: subscription.customer_id,
//             customer_external_id: subscription.customer_external_id,
//             plan_version_id: subscription.plan_version_id,
//             billing_start_date,
//             billing_end_date: subscription
//                 .billing_end_date
//                 .map(date_to_chrono)
//                 .transpose()?,
//             billing_day: subscription.billing_day,
//             effective_billing_period,
//             invoice_date: invoice_date.clone(),
//             current_period_idx: calculate_period_idx(
//                 billing_start_date.clone(),
//                 subscription.billing_day as u32,
//                 invoice_date.clone(),
//                 effective_billing_period,
//             ),
//             currency: subscription.currency,
//             net_terms: subscription.net_terms,
//             parameters,
//             schedule,
//             price_components: resolved_price_components,
//         })
//     }
// }
