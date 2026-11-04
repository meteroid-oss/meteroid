use std::time::Duration;

use meteroid_grpc::meteroid::portal::purchase::v1::purchase_service_server::PurchaseService;
use meteroid_grpc::meteroid::portal::purchase::v1::{
    BuyerType, GetPurchaseOfferRequest, GetPurchaseOfferResponse, OfferComponent, PurchaseOffer,
    StartPurchaseRequest, StartPurchaseResponse, TaxIdCollection as ServerTaxIdCollection,
};
use meteroid_store::domain::purchase_links::{AllowedBuyerTypes, InstanceId, TaxIdCollection};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterfaceAuto;
use tonic::{Request, Response, Status};

use super::PurchaseServiceComponents;
use crate::portal::branding::resolve_portal_branding;
use crate::subscriptions::mapping::price_components::{
    subscription_fee_billing_period_to_grpc, subscription_fee_to_grpc,
};
use meteroid_app::services::purchase_links::{BillingAddressInput, Buyer, StartPurchase};
use meteroid_app::services::storage;

#[tonic::async_trait]
impl PurchaseService for PurchaseServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_purchase_offer(
        &self,
        request: Request<GetPurchaseOfferRequest>,
    ) -> Result<Response<GetPurchaseOfferResponse>, Status> {
        let req = request.into_inner();
        let offer = self
            .purchases
            .get_offer(&req.token, req.license_request.as_deref())
            .await?;

        let tenant_id = offer.link.link.tenant_id;
        let seller = self
            .store
            .get_invoicing_entity(tenant_id, None)
            .await
            .map_err(|e| {
                log::error!("Purchase offer without a default invoicing entity: {e:?}");
                Status::internal("Internal error")
            })?;
        let branding = resolve_portal_branding(&self.store, tenant_id, &seller)
            .await
            .ok();
        let seller_logo_url = match branding.as_ref().and_then(|b| b.logo_attachment_id) {
            Some(id) => storage::signed_url(
                self.object_store.as_ref(),
                &self.store,
                id,
                Duration::from_secs(86400),
            )
            .await
            .ok()
            .flatten(),
            None => None,
        };

        let link = offer.link;
        let buyer_types = match link.link.allowed_buyer_types {
            AllowedBuyerTypes::All => vec![BuyerType::Company, BuyerType::Individual],
            AllowedBuyerTypes::CompaniesOnly => vec![BuyerType::Company],
            AllowedBuyerTypes::IndividualsOnly => vec![BuyerType::Individual],
        };
        let tax_id_collection = match link.link.tax_id_collection {
            TaxIdCollection::Off => ServerTaxIdCollection::Off,
            TaxIdCollection::Optional => ServerTaxIdCollection::Optional,
            TaxIdCollection::Required => ServerTaxIdCollection::Required,
        };
        Ok(Response::new(GetPurchaseOfferResponse {
            offer: Some(PurchaseOffer {
                seller_name: seller.legal_name,
                seller_logo_url,
                seller_brand_color: branding.and_then(|b| b.brand_color),
                plan_name: link.plan.name,
                plan_description: link.plan.description,
                currency: link.version.currency,
                trial_days: link
                    .version
                    .trial_duration_days
                    .and_then(|d| u32::try_from(d).ok())
                    .filter(|d| *d > 0),
                components: offer
                    .components
                    .into_iter()
                    .map(|c| OfferComponent {
                        name: c.name,
                        period: subscription_fee_billing_period_to_grpc(c.period).into(),
                        fee: Some(subscription_fee_to_grpc(
                            &c.fee,
                            c.period.as_billing_period_opt().unwrap_or_default(),
                        )),
                    })
                    .collect(),
                allow_coupon_codes: link.link.allow_coupon_codes,
                pow_challenge: offer.pow_challenge,
                for_installation: offer.installation_id.is_some(),
                installation_id: offer.installation_id.map(InstanceId::into_string),
                buyer_types: buyer_types.into_iter().map(Into::into).collect(),
                require_billing_address: link.link.require_billing_address
                    || seller.require_billing_information,
                require_billing_information: seller.require_billing_information,
                tax_id_collection: tax_id_collection.into(),
                tax_inclusive: link.version.tax_inclusive,
            }),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn start_purchase(
        &self,
        request: Request<StartPurchaseRequest>,
    ) -> Result<Response<StartPurchaseResponse>, Status> {
        let client_ip = meteroid_app::security_context::from_request(&request).ip_address;
        let req = request.into_inner();

        let buyer = match BuyerType::try_from(req.buyer_type) {
            Ok(BuyerType::Company) => Buyer::Company {
                name: req.company_name.unwrap_or_default(),
            },
            Ok(BuyerType::Individual) => Buyer::Individual {
                first_name: req.first_name.unwrap_or_default(),
                last_name: req.last_name.unwrap_or_default(),
            },
            Err(_) => return Err(Status::invalid_argument("Unknown buyer type")),
        };
        let address = req
            .billing_address
            .map(|a| BillingAddressInput {
                line1: a.line1,
                line2: a.line2,
                city: a.city,
                state: a.state,
                zip_code: a.zip_code,
            })
            .unwrap_or_default();

        let started = self
            .purchases
            .start(StartPurchase {
                token: req.token,
                email: req.email,
                buyer,
                country: req.country,
                address,
                vat_number: req.vat_number,
                legal_number: req.legal_number,
                pow_solution: req.pow_solution,
                attempt_id: req.attempt_id,
                license_request: req.license_request,
                client_ip,
            })
            .await?;

        Ok(Response::new(StartPurchaseResponse {
            checkout_url: started.checkout_url,
        }))
    }
}
