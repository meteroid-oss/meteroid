//! Per-environment policy maps.
//!
//! A policy map is a flat JSON object — the shape a Config/MAP entitlement carries —
//! whose keys are tenant environments and whose values apply to tenants in that
//! environment:
//!
//! ```json
//! { "default": true, "production": false }
//! ```
//!
//! `default` (or `*`) covers every environment not named explicitly, so the common
//! "all but production" policy is two keys instead of six. Keys are matched
//! case-insensitively and accept the short slug form (`dev` for `development`).

use serde_json::Value;

use crate::domain::enums::TenantEnvironmentEnum;

/// Key applying to every environment not named explicitly. `*` is accepted too.
const DEFAULT_KEYS: [&str; 2] = ["default", "*"];

/// Canonical key for an environment, plus the short slug form as an alias.
fn environment_keys(environment: &TenantEnvironmentEnum) -> [&'static str; 2] {
    match environment {
        TenantEnvironmentEnum::Production => ["production", "prod"],
        TenantEnvironmentEnum::Staging => ["staging", "stg"],
        TenantEnvironmentEnum::Qa => ["qa", "qa"],
        TenantEnvironmentEnum::Development => ["development", "dev"],
        TenantEnvironmentEnum::Sandbox => ["sandbox", "sand"],
        TenantEnvironmentEnum::Demo => ["demo", "demo"],
    }
}

/// Case-insensitive lookup of any of `keys` in a JSON object.
fn lookup<'a>(map: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let object = map.as_object()?;
    object.iter().find_map(|(k, v)| {
        let k = k.trim().to_ascii_lowercase();
        keys.iter().any(|want| k == *want).then_some(v)
    })
}

/// The value a policy map assigns to `environment`: its own entry when present,
/// otherwise the map's `default`. `None` when the map names neither (or isn't an
/// object), leaving the built-in fallback to the caller.
pub fn environment_value<'a>(
    map: &'a Value,
    environment: &TenantEnvironmentEnum,
) -> Option<&'a Value> {
    lookup(map, &environment_keys(environment)).or_else(|| lookup(map, &DEFAULT_KEYS))
}

/// `environment_value` read as a flag. The map editor stores booleans, numbers and
/// strings, so all three are accepted; anything else reads as `false`.
pub fn environment_flag(map: &Value, environment: &TenantEnvironmentEnum) -> Option<bool> {
    environment_value(map, environment).map(as_flag)
}

fn as_flag(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => matches!(
            s.trim().to_ascii_lowercase().as_str(),
            "true" | "1" | "yes" | "on"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ENVIRONMENTS: [TenantEnvironmentEnum; 6] = [
        TenantEnvironmentEnum::Production,
        TenantEnvironmentEnum::Staging,
        TenantEnvironmentEnum::Qa,
        TenantEnvironmentEnum::Development,
        TenantEnvironmentEnum::Sandbox,
        TenantEnvironmentEnum::Demo,
    ];

    #[test]
    fn an_explicit_entry_wins_over_the_default() {
        let map = json!({ "default": true, "production": false });
        for env in ENVIRONMENTS {
            let expected = env != TenantEnvironmentEnum::Production;
            assert_eq!(environment_flag(&map, &env), Some(expected), "{env:?}");
        }
    }

    #[test]
    fn star_is_a_default_too() {
        let map = json!({ "*": false, "sandbox": true });
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Sandbox),
            Some(true)
        );
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Demo),
            Some(false)
        );
    }

    #[test]
    fn a_map_naming_neither_leaves_the_fallback_to_the_caller() {
        let map = json!({ "sandbox": true });
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Production),
            None
        );
        assert_eq!(
            environment_flag(&json!("nope"), &TenantEnvironmentEnum::Qa),
            None
        );
    }

    #[test]
    fn keys_match_case_insensitively_and_by_slug() {
        let map = json!({ "Production": true, "dev": true });
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Production),
            Some(true)
        );
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Development),
            Some(true)
        );
    }

    #[test]
    fn flags_accept_the_map_editors_value_types() {
        let map = json!({
            "sandbox": "true",
            "development": 1,
            "qa": 0,
            "staging": "no",
            "demo": ["not", "a", "flag"],
        });
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Sandbox),
            Some(true)
        );
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Development),
            Some(true)
        );
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Qa),
            Some(false)
        );
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Staging),
            Some(false)
        );
        assert_eq!(
            environment_flag(&map, &TenantEnvironmentEnum::Demo),
            Some(false)
        );
    }

    #[test]
    fn non_flag_values_are_readable_as_they_are() {
        let map = json!({ "default": 30, "sandbox": "unlimited" });
        assert_eq!(
            environment_value(&map, &TenantEnvironmentEnum::Sandbox),
            Some(&json!("unlimited"))
        );
        assert_eq!(
            environment_value(&map, &TenantEnvironmentEnum::Production),
            Some(&json!(30))
        );
    }
}
