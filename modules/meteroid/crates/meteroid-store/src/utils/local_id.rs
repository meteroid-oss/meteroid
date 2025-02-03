use nanoid::nanoid;

#[derive(Debug)]
pub enum IdType {
    AddOn,
    BankAccount,
    BillableMetric,
    Coupon,
    Customer,
    Invoice,
    InvoicingEntity,
    Other,
    Plan,
    PriceComponent,
    Product,
    ProductFamily,
    Subscription,
    Tenant,
    Event,
}

impl IdType {
    fn prefix(&self) -> &'static str {
        match self {
            IdType::AddOn => "add_",
            IdType::BankAccount => "ba_",
            IdType::BillableMetric => "bm_",
            IdType::Coupon => "cou_",
            IdType::Customer => "cus_",
            IdType::Event => "evt_",
            IdType::Invoice => "inv_",
            IdType::InvoicingEntity => "ive_",
            IdType::Plan => "plan_",
            IdType::PriceComponent => "price_",
            IdType::Product => "prd_",
            IdType::ProductFamily => "pf_",
            IdType::Subscription => "sub_",
            IdType::Tenant => "",
            _ => "",
        }
    }
}

/**
 * Generates a local id for a given type. Local ids are small human-readable ids for the API, unique per tenant
 */
pub struct LocalId;

impl LocalId {
    const ID_LENGTH: usize = 13;
    fn generate_local_id(prefix: &str, length: usize) -> String {
        let id = nanoid!(length, &common_utils::rng::BASE62_ALPHABET);
        format!("{}{}", prefix, id)
    }
    pub fn generate_for(id_type: IdType) -> String {
        let prefix = id_type.prefix();
        Self::generate_local_id(prefix, Self::ID_LENGTH)
    }

    pub fn no_prefix() -> String {
        let prefix = IdType::Other.prefix();
        Self::generate_local_id(prefix, Self::ID_LENGTH)
    }

    pub fn generate_custom(prefix: &str, length: usize) -> String {
        Self::generate_local_id(prefix, length)
    }
}
