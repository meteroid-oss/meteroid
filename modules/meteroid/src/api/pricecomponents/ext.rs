// use cornucopia_async::{GenericClient, Params};
// use meteroid_grpc::meteroid::api::components::v1::fee::capacity::capacity_pricing::Pricing as CapacityPricing;
// use meteroid_grpc::meteroid::api::components::v1::fee::r#type::Fee;
// use meteroid_grpc::meteroid::api::components::v1::fee::term_fee_pricing::Pricing;
// use meteroid_grpc::meteroid::api::components::v1::PriceComponent;
// use meteroid_repository as db;
// use std::sync::Arc;
//
// use tonic::Status;
// use uuid::Uuid;
//
// use super::mapping;
//
// pub enum PlanParameter {
//     BillingPeriodTerm,
//     CapacityThresholdValue {
//         component_id: String,
//         capacity_values: Vec<u64>,
//     },
//     CommittedSlot {
//         component_id: String,
//     },
// }
//
// pub async fn list_price_components<C: GenericClient>(
//     plan_version_id: Uuid,
//     tenant_id: Uuid,
//     connection: &C,
// ) -> Result<Vec<PriceComponent>, Status> {
//     let res = db::price_components::list_price_components()
//         .params(
//             connection,
//             &db::price_components::ListPriceComponentsParams {
//                 plan_version_id,
//                 tenant_id,
//             },
//         )
//         .all()
//         .await
//         .map_err(|e| {
//             Status::internal("Unable to list price components")
//                 .set_source(Arc::new(e))
//                 .clone()
//         })?;
//
//     let components = res
//         .into_iter()
//         .map(mapping::components::db_to_server)
//         .collect::<Result<Vec<PriceComponent>, Status>>()?;
//
//     Ok(components)
// }
//
// pub fn components_to_params(components: Vec<PriceComponent>) -> Vec<PlanParameter> {
//     let mut parameters: Vec<PlanParameter> = vec![];
//     let mut billing_term = false;
//
//     for component in components {
//         if let Some(fee) = component.fee_type.and_then(|fee_type| fee_type.fee) {
//             match fee {
//                 Fee::Rate(rate) => {
//                     if let Some(Pricing::TermBased(_)) = rate.pricing.and_then(|p| p.pricing) {
//                         billing_term = true;
//                     }
//                 }
//                 Fee::SlotBased(rate) => {
//                     if let Some(pricing) = rate.pricing.and_then(|p| p.pricing) {
//                         parameters.push(PlanParameter::CommittedSlot {
//                             component_id: component.id,
//                         });
//                         if let Pricing::TermBased(_) = pricing {
//                             billing_term = true;
//                         }
//                     }
//                 }
//                 Fee::Capacity(cap) => {
//                     if let Some(pricing) = cap.pricing.and_then(|p| p.pricing) {
//                         match pricing {
//                             CapacityPricing::TermBased(c) => {
//                                 billing_term = true;
//                                 parameters.push(PlanParameter::CapacityThresholdValue {
//                                     component_id: component.id,
//                                     capacity_values: c
//                                         .rates
//                                         .first()
//                                         .map(|r| {
//                                             r.thresholds.iter().map(|t| t.included_amount).collect()
//                                         })
//                                         .unwrap_or(vec![]),
//                                 });
//                             }
//                             CapacityPricing::Single(c) => {
//                                 parameters.push(PlanParameter::CapacityThresholdValue {
//                                     component_id: component.id,
//                                     capacity_values: c
//                                         .thresholds
//                                         .iter()
//                                         .map(|t| t.included_amount)
//                                         .collect(),
//                                 });
//                             }
//                         }
//                     }
//                 }
//                 _ => {}
//             }
//         }
//     }
//     if billing_term {
//         parameters.push(PlanParameter::BillingPeriodTerm);
//     }
//
//     parameters
// }
