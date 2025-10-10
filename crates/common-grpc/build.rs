fn main() {
    let mut config = prost_build::Config::new();
    config.btree_map(["."]);
    config.protoc_arg("--experimental_allow_proto3_optional");

    config.type_attribute(
        ".meteroid.common.v1.Decimal",
        "#[derive(serde::Serialize, serde::Deserialize)]",
    );
    config.type_attribute(
        ".meteroid.common.v1.Date",
        "#[derive(serde::Serialize, serde::Deserialize)]",
    );

    config
        .compile_protos(
            &[
                "proto/common/v1/date.proto",
                "proto/common/v1/decimal.proto",
                "proto/common/v1/pagination.proto",
            ],
            &["."],
        )
        .unwrap_or_else(|e| panic!("{e}"));
}
