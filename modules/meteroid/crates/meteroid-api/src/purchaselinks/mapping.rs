use meteroid_grpc::meteroid::api::purchaselinks::v1 as server;
use meteroid_store::domain::purchase_links::{
    AllowedBuyerTypes, PurchaseLinkWithStats, TaxIdCollection,
};

use meteroid_app::conversions::{AsProtoOpt, ProtoConv};

pub fn purchase_link_url(public_url: &str, token: &str) -> String {
    format!("{public_url}/purchase/{token}")
}

pub fn domain_to_server(link: PurchaseLinkWithStats, public_url: &str) -> server::PurchaseLink {
    let PurchaseLinkWithStats {
        link,
        plan_name,
        started_count,
        completed_count,
    } = link;
    server::PurchaseLink {
        id: link.id.as_proto(),
        plan_id: link.plan_id.as_proto(),
        plan_name,
        name: link.name,
        url: purchase_link_url(public_url, &link.token),
        allow_coupon_codes: link.allow_coupon_codes,
        listed: link.listed,
        success_url: link.success_url,
        cancel_url: link.cancel_url,
        created_at: link.created_at.as_proto(),
        archived_at: link.archived_at.as_proto(),
        started_count,
        completed_count,
        require_billing_address: link.require_billing_address,
        tax_id_collection: tax_id_collection_to_server(link.tax_id_collection).into(),
        allowed_buyer_types: allowed_buyer_types_to_server(link.allowed_buyer_types).into(),
    }
}

pub fn allowed_buyer_types_to_server(value: AllowedBuyerTypes) -> server::AllowedBuyerTypes {
    match value {
        AllowedBuyerTypes::All => server::AllowedBuyerTypes::All,
        AllowedBuyerTypes::CompaniesOnly => server::AllowedBuyerTypes::CompaniesOnly,
        AllowedBuyerTypes::IndividualsOnly => server::AllowedBuyerTypes::IndividualsOnly,
    }
}

pub fn allowed_buyer_types_from_server(value: server::AllowedBuyerTypes) -> AllowedBuyerTypes {
    match value {
        server::AllowedBuyerTypes::All => AllowedBuyerTypes::All,
        server::AllowedBuyerTypes::CompaniesOnly => AllowedBuyerTypes::CompaniesOnly,
        server::AllowedBuyerTypes::IndividualsOnly => AllowedBuyerTypes::IndividualsOnly,
    }
}

pub fn tax_id_collection_to_server(value: TaxIdCollection) -> server::TaxIdCollection {
    match value {
        TaxIdCollection::Off => server::TaxIdCollection::Off,
        TaxIdCollection::Optional => server::TaxIdCollection::Optional,
        TaxIdCollection::Required => server::TaxIdCollection::Required,
    }
}

pub fn tax_id_collection_from_server(value: server::TaxIdCollection) -> TaxIdCollection {
    match value {
        server::TaxIdCollection::Off => TaxIdCollection::Off,
        server::TaxIdCollection::Optional => TaxIdCollection::Optional,
        server::TaxIdCollection::Required => TaxIdCollection::Required,
    }
}
