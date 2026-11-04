use tonic::transport::Channel;

use meteroid_grpc::meteroid::api::users::v1::LoginResponse;

use super::clients::AllClients;

pub const SEED_USERNAME: &str = "demo-user@meteroid.dev";
pub const SEED_PASSWORD: &str = "sandbox-F3j";

pub async fn login(channel: Channel) -> LoginResponse {
    // for auth we don't have yet token and slug
    AllClients::from_channel(channel, "", "", "")
        .users
        .clone()
        .login(tonic::Request::new(
            meteroid_grpc::meteroid::api::users::v1::LoginRequest {
                email: SEED_USERNAME.to_string(),
                password: SEED_PASSWORD.to_string(),
            },
        ))
        .await
        .unwrap()
        .into_inner()
}
