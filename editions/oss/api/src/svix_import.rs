#[tokio::main]
async fn main() -> std::process::ExitCode {
    meteroid::svix_import::run(meteroid_edition_oss::modules).await
}
