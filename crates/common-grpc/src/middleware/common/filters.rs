pub type Filter = fn(&str) -> bool;

pub fn reject_healthcheck(path: &str) -> bool {
    !path.contains("grpc.health.") //"grpc.health.v1.Health"
}

pub fn only_internal(path: &str) -> bool {
    path.starts_with("/meteroid.internal.")
}

pub fn only_api(path: &str) -> bool {
    path.starts_with("/meteroid.api.")
}

pub fn only_portal(path: &str) -> bool {
    path.starts_with("/meteroid.portal.")
}
