pub mod address {
    use crate::api_rest::addresses::model::Address;
    use meteroid_store::domain;

    pub fn domain_to_rest(d: domain::Address) -> Address {
        Address {
            line1: d.line1,
            line2: d.line2,
            city: d.city,
            country: d.country,
            state: d.state,
            zip_code: d.zip_code,
        }
    }
}

pub mod shipping_address {
    use crate::api_rest::addresses::mapping;
    use crate::api_rest::addresses::model::ShippingAddress;
    use meteroid_store::domain;

    pub fn domain_to_rest(d: domain::ShippingAddress) -> ShippingAddress {
        ShippingAddress {
            address: d.address.map(mapping::address::domain_to_rest),
            same_as_billing: d.same_as_billing,
        }
    }
}
