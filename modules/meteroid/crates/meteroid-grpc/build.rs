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
        "activation",
        "activity",
        "addons",
        "alerts",
        "analytics",
        "apitokens",
        "customproperties",
        "bankaccounts",
        "batchjobs",
        "billablemetrics",
        "budgets",
        "connect",
        "connectors",
        "consolidation",
        "creditunits",
        "entitlements",
        "customers",
        "coupons",
        "creditnotes",
        "customeremails",
        "domains",
        "events",
        "eventinsights",
        "features",
        "instance",
        "invoices",
        "invoiceannextemplates",
        "licenses",
        "licensing",
        "listviews",
        "invoicingentities",
        "organizations",
        "paymentreminders",
        "plans",
        "pricecomponents",
        "prices",
        "notifications",
        "productfamilies",
        "products",
        "purchaselinks",
        "quotes",
        "rbac",
        "savedviews",
        "schedules",
        "security",
        "segments",
        "signals",
        "sso",
        "stats",
        "subscriptions",
        "targets",
        "taxes",
        "taxreports",
        "tenants",
        "users",
        "walletcontrols",
        "wallets",
        "webhooksout",
    ];

    let mut proto_files = Vec::new();
    for service in &services {
        let service_path = root.join(format!("proto/api/{service}/v1"));
        proto_files.push(service_path.join(format!("{service}.proto"))); // main service file
        // Watch model files imported by service protos
        let models_path = service_path.join("models.proto");
        if models_path.exists() {
            println!("cargo:rerun-if-changed={}", models_path.display());
        }
    }
    // Internal service-to-service protos
    proto_files.push(root.join("proto/internal/v1/internal.proto"));
    proto_files.push(root.join("proto/portal/checkout/v1/checkout.proto"));
    proto_files.push(root.join("proto/portal/purchase/v1/purchase.proto"));
    proto_files.push(root.join("proto/portal/invoice/v1/invoice.proto"));
    proto_files.push(root.join("proto/portal/shared/v1/shared.proto"));
    proto_files.push(root.join("proto/portal/customer/v1/customer.proto"));
    proto_files.push(root.join("proto/portal/quotes/v1/quotes.proto"));
    proto_files.push(root.join("proto/portal/subscription/v1/subscription.proto"));
    proto_files.push(root.join("proto/portal/dashboard/v1/dashboard.proto"));
    proto_files.push(root.join("proto/portal/wallets/v1/wallets.proto"));
    proto_files.push(root.join("proto/portal/walletcontrols/v1/walletcontrols.proto"));
    proto_files.push(root.join("proto/portal/budgets/v1/budgets.proto"));

    for proto_file in &proto_files {
        println!("cargo:rerun-if-changed={}", proto_file.display());
        // Watch the sibling models.proto imported by each service proto, so edits
        // to message definitions (not just the service file) retrigger codegen.
        let models_path = proto_file.with_file_name("models.proto");
        if models_path != *proto_file && models_path.exists() {
            println!("cargo:rerun-if-changed={}", models_path.display());
        }
    }

    let out_dir = PathBuf::from(
        env::var("OUT_DIR")
            .change_context(BuildError)
            .attach("Failed to retrieve OUT_DIR environment variable")?,
    );

    let descriptor_path = out_dir.join("meteroid-grpc.protoset.bin");

    tonic_prost_build::configure()
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
                root.join("proto"),
                root.join("../../crates/common-grpc/proto"),
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
