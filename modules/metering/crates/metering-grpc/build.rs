use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let proto_root = root.join("proto");
    let common_proto_root = root.join("../../crates/common-grpc/proto");

    // Automatically discover all .proto files in the proto directory
    let proto_files: Vec<PathBuf> = fs::read_dir(&proto_root)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension()? == "proto" {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    // Tell Cargo to rerun this build script when proto files change
    for proto_file in &proto_files {
        println!("cargo:rerun-if-changed={}", proto_file.display());
    }

    // Convert absolute paths to relative paths for compile_protos
    // Exclude models.proto as it's imported by other proto files and shouldn't be compiled directly
    let proto_file_paths: Vec<String> = proto_files
        .iter()
        .filter_map(|path| {
            let file_name = path.file_name()?.to_str()?;
            // Skip models.proto as it's a shared dependency
            if file_name == "models.proto" {
                return None;
            }
            path.strip_prefix(&root)
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.to_string())
        })
        .collect();

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .client_mod_attribute(".", r#"#[cfg(feature = "client")]"#)
        .server_mod_attribute(".", r#"#[cfg(feature = "server")]"#)
        .extern_path(".meteroid.common", "::common_grpc::meteroid::common")
        .protoc_arg("--experimental_allow_proto3_optional")
        .compile_protos(
            &proto_file_paths
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            &[
                root.to_str().unwrap(),
                proto_root.to_str().unwrap(),
                common_proto_root.to_str().unwrap(),
            ],
        )?;

    Ok(())
}
