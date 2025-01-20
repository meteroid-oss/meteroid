use error_stack::{Result, ResultExt};
use std::env;
use std::path::{Path, PathBuf};
use thiserror::Error;

fn main() -> Result<(), BuildError> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    generate_grpc_types(&root)?;

    Ok(())
}

fn generate_grpc_types(root: &Path) -> Result<(), BuildError> {
    let services = vec![
        "addons",
        "apitokens",
        "bankaccounts",
        "billablemetrics",
        "customers",
        "coupons",
        "instance",
        "invoices",
        "invoicingentities",
        "organizations",
        "plans",
        "pricecomponents",
        "productfamilies",
        "products",
        "schedules",
        "stats",
        "subscriptions",
        "tenants",
        "users",
        "webhooksout",
    ];

    let mut proto_files = Vec::new();
    for service in services {
        let service_path = root.join(format!("proto/api/{}/v1", service));
        proto_files.push(service_path.join(format!("{}.proto", service))); // main service file
    }
    // Add additional paths as needed
    proto_files.push(root.join("proto/internal/v1/internal.proto"));
    proto_files.push(root.join("proto/api/subscriptions/v1/subscriptions.proto"));
    proto_files.push(root.join("proto/api/pricecomponents/v1/pricecomponents.proto"));

    for proto_file in &proto_files {
        println!("cargo:rerun-if-changed={}", proto_file.display());
    }

    let out_dir = PathBuf::from(
        env::var("OUT_DIR")
            .change_context(BuildError)
            .attach_printable("Failed to retrieve OUT_DIR environment variable")?,
    );

    let descriptor_path = out_dir.join("meteroid-grpc.protoset.bin");

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .client_mod_attribute(".", r#"#[cfg(feature = "client")]"#)
        .server_mod_attribute(".", r#"#[cfg(feature = "server")]"#)
        .extern_path(".meteroid.common", "::common_grpc::meteroid::common")
        .file_descriptor_set_path(descriptor_path.clone())
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile_protos(
            &proto_files,
            &[
                root,
                &root.join("proto"),
                &root.join("../../crates/common-grpc/proto"),
            ],
        )
        .change_context(BuildError)
        .attach_printable("Failed to compile protobuf files")?;

    let serde_paths = &[
        ".meteroid.api.billablemetrics.v1.segmentation_matrix",
        ".meteroid.api.billablemetrics.v1.SegmentationMatrix",
        ".meteroid.api.components.v1",
        ".meteroid.api.shared.v1",
        ".meteroid.api.adjustments.v1",
        ".meteroid.api.tenants.v1.TenantBillingConfiguration",
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
