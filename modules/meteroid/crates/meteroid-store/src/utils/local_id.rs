use nanoid::nanoid;

#[derive(Debug)]
pub enum IdType {
    Other,
}

impl IdType {
    fn prefix(&self) -> &'static str {
        ""
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
