use error_stack::{Report, ResultExt};
use std::env;
use std::path::{Path, PathBuf};
use thiserror::Error;

fn main() -> Result<(), Report<BuildError>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    generate_grpc_types(&root)?;

    Ok(())
}

fn generate_grpc_types(root: &Path) -> Result<(), Report<BuildError>> {
    let services = vec![
        "addons",
        "apitokens",
        "bankaccounts",
        "billablemetrics",
        "connectors",
        "customers",
        "coupons",
        "events",
        "instance",
        "invoices",
        "invoicingentities",
        "organizations",
        "plans",
        "pricecomponents",
        "productfamilies",
        "products",
        "quotes",
        "schedules",
        "stats",
        "subscriptions",
        "taxes",
        "tenants",
        "users",
        "webhooksout",
    ];

    let mut proto_files = Vec::new();
    for service in services {
        let service_path = root.join(format!("proto/api/{service}/v1"));
        proto_files.push(service_path.join(format!("{service}.proto"))); // main service file
    }
    // Add additional paths as needed
    proto_files.push(root.join("proto/internal/v1/internal.proto"));
    proto_files.push(root.join("proto/portal/checkout/v1/checkout.proto"));
    proto_files.push(root.join("proto/portal/quotes/v1/quotes.proto"));

    for proto_file in &proto_files {
        println!("cargo:rerun-if-changed={}", proto_file.display());
    }

    let out_dir = PathBuf::from(
        env::var("OUT_DIR")
            .change_context(BuildError)
            .attach("Failed to retrieve OUT_DIR environment variable")?,
    );

    let descriptor_path = out_dir.join("meteroid-grpc.protoset.bin");

    let proto_root = root.join("proto");
    let common_proto_root = root.join("../../crates/common-grpc/proto");

    let proto_file_refs: Vec<&str> = proto_files.iter().map(|p| p.to_str().unwrap()).collect();

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .client_mod_attribute(".", r#"#[cfg(feature = "client")]"#)
        .server_mod_attribute(".", r#"#[cfg(feature = "server")]"#)
        .extern_path(".meteroid.common", "::common_grpc::meteroid::common")
        .file_descriptor_set_path(descriptor_path.clone())
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile_protos(
            &proto_file_refs,
            &[
                root.to_str().unwrap(),
                proto_root.to_str().unwrap(),
                common_proto_root.to_str().unwrap(),
            ],
        )
        .change_context(BuildError)
        .attach("Failed to compile protobuf files")?;

    let serde_paths = &[
        ".meteroid.api.billablemetrics.v1.segmentation_matrix",
        ".meteroid.api.billablemetrics.v1.SegmentationMatrix",
        ".meteroid.api.components.v1",
        ".meteroid.api.shared.v1",
        ".meteroid.api.adjustments.v1",
        ".meteroid.api.schedules.v1.PlanRamps",
        ".meteroid.api.customers.v1.CustomerBillingConfig",
        ".meteroid.api.customers.v1.Address",
        ".meteroid.api.customers.v1.ShippingAddress",
        ".meteroid.api.subscriptions.v1.SubscriptionParameters",
    ];

    let descriptor_set = std::fs::read(descriptor_path.clone()).change_context(BuildError)?;

    // generates serde impl matching the proto json spec, so with the same guarantees
    pbjson_build::Builder::new()
        .register_descriptors(&descriptor_set)
        .change_context(BuildError)?
        .build(serde_paths)
        .change_context(BuildError)?;

    Ok(())
}

#[derive(Debug, Error)]
#[error("Build Error")]
pub struct BuildError;
