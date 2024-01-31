pub fn v4() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

// while uuid crate officialy supports v7, TODO
pub fn v7() -> uuid::Uuid {
    uuid::Uuid::from_bytes(*uuid7::uuid7().as_bytes())
}
