// TODO remove this now that Uuid officially supports v7

pub fn v4() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}

pub fn v7() -> uuid::Uuid {
    uuid::Uuid::now_v7()
}
