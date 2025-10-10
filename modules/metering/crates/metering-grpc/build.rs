use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let proto_root = root.join("proto");
    let common_proto_root = root.join("../../crates/common-grpc/proto");

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .client_mod_attribute(".", r#"#[cfg(feature = "client")]"#)
        .server_mod_attribute(".", r#"#[cfg(feature = "server")]"#)
        .extern_path(".meteroid.common", "::common_grpc::meteroid::common")
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile_protos(
            &[
                "proto/events.proto",
                "proto/events_internal.proto",
                "proto/meters.proto",
                "proto/queries.proto",
            ],
            &[
                root.to_str().unwrap(),
                proto_root.to_str().unwrap(),
                common_proto_root.to_str().unwrap(),
            ],
        )?;

    Ok(())
}
