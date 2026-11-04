use common_build_info::BuildInfo;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let build_info = BuildInfo::set(env!("CARGO_BIN_NAME"));
    println!("Starting {build_info}");
    meteroid_workers::run::run_workers(meteroid_edition_oss::modules).await
}
