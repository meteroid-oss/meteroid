//! REST and gRPC map from one store type per operation (ADR 0011 §5): their fields, pairwise.
use crate::openapi::{EditionSpec, published_spec_root};
use prost::Message;
use prost_types::FileDescriptorSet;

/// A REST schema, a proto message, and their fields as `(rest, grpc)`.
type Pair = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

const E: &str = "meteroid.api.entitlements.v1";
const R: &str = "meteroid.api.resources.v1";

type Fields = Vec<(&'static str, &'static str)>;

fn same(fields: &'static [&'static str]) -> Fields {
    fields.iter().map(|f| (*f, *f)).collect()
}

fn pairs() -> Vec<(&'static str, String, Fields)> {
    let renamed: &[Pair] = &[
        ("Feature", "Feature", &[("level", "level_resource_type_id")]),
        (
            "EntitlementHistoryResponse",
            "ListEntitlementHistoryResponse",
            &[("data", "entitlements")],
        ),
        (
            "EntitlementCheck",
            "CheckEntitlementResponse",
            &[("retry_after_ms", "retry_after")],
        ),
        (
            "EntitlementLevelState",
            "EntitlementLevelState",
            &[("level", "resource_type_id")],
        ),
        (
            "SnapshotLevel",
            "SnapshotLevel",
            &[("level", "resource_type_id")],
        ),
        (
            "EntitlementSnapshot",
            "GetEntitlementSnapshotResponse",
            &[("data", "features")],
        ),
    ];
    let shared: &[(&str, &str, &str, &'static [&'static str])] = &[
        (
            "Feature",
            E,
            "Feature",
            &[
                "id",
                "name",
                "code",
                "description",
                "kind",
                "value",
                "limit",
                "unit",
                "policy",
                "feature_type",
                "status",
                "product",
                "created_at",
                "entitlement",
                "defaults",
            ],
        ),
        (
            "Entitlement",
            E,
            "Entitlement",
            &[
                "id",
                "feature_id",
                "value",
                "effect",
                "level",
                "level_type_id",
                "resource_id",
                "condition",
                "valid_from",
                "valid_until",
                "reason",
                "policy_override",
                "created_by",
                "created_at",
                "updated_at",
                "revoked_at",
                "revoked_by",
            ],
        ),
        (
            "EntitlementSpecRequest",
            E,
            "EntitlementSpec",
            &[
                "feature_id",
                "value",
                "effect",
                "level",
                "level_type_id",
                "condition",
                "valid_from",
                "valid_until",
                "reason",
                "policy_override",
            ],
        ),
        (
            "FeaturePolicy",
            E,
            "FeaturePolicy",
            &["resets", "period", "at_limit", "grace_pct", "thresholds"],
        ),
        (
            "EntitlementCheckRequest",
            E,
            "CheckEntitlementRequest",
            &[
                "feature",
                "resources",
                "attributes",
                "requested",
                "current_count",
                "explain",
                "subscription_id",
            ],
        ),
        (
            "EntitlementCheck",
            E,
            "CheckEntitlementResponse",
            &[
                "feature",
                "allowed",
                "reason",
                "reason_type",
                "levels",
                "hint",
                "attributes_ignored",
                "warnings",
                "usage_source",
                "near_limit",
                "in_overage",
                "value",
            ],
        ),
        (
            "EntitlementLevelState",
            E,
            "EntitlementLevelState",
            &[
                "path",
                "value",
                "used",
                "limit",
                "near_limit",
                "in_overage",
                "reset_at",
                "deciding_subscription_id",
                "allowed",
                "reason",
                "explain",
            ],
        ),
        (
            "SnapshotLevel",
            E,
            "SnapshotLevel",
            &[
                "path",
                "value",
                "reason",
                "policy",
                "used",
                "reset_at",
                "deciding_subscription_id",
                "origin",
            ],
        ),
        (
            "EntitlementContribution",
            E,
            "Contribution",
            &[
                "entitlement_id",
                "source",
                "source_name",
                "tier",
                "effect",
                "value",
                "condition",
                "quantity",
                "units",
                "policy_override",
                "running",
                "discarded",
            ],
        ),
        (
            "LevelExplain",
            E,
            "LevelExplain",
            &["contributions", "candidates", "app_counts"],
        ),
        (
            "SnapshotEntitlement",
            E,
            "SnapshotFeature",
            &["feature", "scope", "value", "levels"],
        ),
        (
            "EntitlementSnapshot",
            E,
            "GetEntitlementSnapshotResponse",
            &["etag", "attributes_ignored", "warnings"],
        ),
        (
            "ReportUsageRequest",
            E,
            "ReportEntitlementUsageRequest",
            &["set", "add", "resources", "at", "idempotency_key"],
        ),
        (
            "ReportUsageResponse",
            E,
            "ReportEntitlementUsageResponse",
            &["path", "count", "applied", "replayed"],
        ),
        (
            "EffectiveEntitlement",
            E,
            "EffectiveEntitlement",
            &["feature", "own_grant"],
        ),
        (
            "SubscriptionGrant",
            E,
            "SubscriptionGrant",
            &[
                "value",
                "deciding_subscription_id",
                "deciding_plan_name",
                "deciding_resource_path",
            ],
        ),
        ("ResourceType", R, "ResourceType", &["archived"]),
        ("Resource", R, "Resource", &["name", "archived"]),
    ];
    shared
        .iter()
        .map(|(rest, package, grpc, fields)| (*rest, format!("{package}.{grpc}"), same(fields)))
        .chain(
            renamed
                .iter()
                .map(|(rest, grpc, fields)| (*rest, format!("{E}.{grpc}"), fields.to_vec())),
        )
        .collect()
}

fn proto_fields(set: &FileDescriptorSet, full_name: &str) -> Vec<String> {
    set.file
        .iter()
        .flat_map(|file| {
            file.message_type
                .iter()
                .map(move |message| (format!("{}.{}", file.package(), message.name()), message))
        })
        .find(|(name, _)| name == full_name)
        .unwrap_or_else(|| panic!("no proto message {full_name}"))
        .1
        .field
        .iter()
        .map(|f| f.name().to_owned())
        .collect()
}

#[test]
fn rest_and_grpc_carry_the_same_fields() {
    let set = FileDescriptorSet::decode(meteroid_grpc::_reflection::FILE_DESCRIPTOR_SET)
        .expect("decode descriptor set");
    let root = published_spec_root(&EditionSpec::core());
    let schemas = &root["components"]["schemas"];
    for (rest, grpc, fields) in pairs() {
        let properties = schemas[rest]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("no REST schema {rest}"));
        let message = proto_fields(&set, &grpc);
        for (rest_field, grpc_field) in fields {
            assert!(properties.contains_key(rest_field), "{rest}.{rest_field}");
            assert!(
                message.iter().any(|f| f == grpc_field),
                "{grpc}.{grpc_field}, paired with {rest}.{rest_field}"
            );
        }
    }
}

/// The resources list filters by type on both: `type` in the query, `resource_type` in gRPC.
#[test]
fn the_resource_type_filter_is_on_both() {
    let set = FileDescriptorSet::decode(meteroid_grpc::_reflection::FILE_DESCRIPTOR_SET)
        .expect("decode descriptor set");
    let root = published_spec_root(&EditionSpec::core());
    let parameters =
        root["paths"]["/api/v1/customers/{id_or_alias}/resources"]["get"]["parameters"]
            .as_array()
            .expect("parameters");
    assert!(
        parameters
            .iter()
            .any(|p| p["name"] == "type" && p["in"] == "query")
    );
    assert!(
        proto_fields(&set, &format!("{R}.ListResourcesRequest"))
            .contains(&"resource_type".to_owned())
    );
}
