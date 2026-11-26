//! The extractors of v2: axum's, refusing a request with a problem.
//!
//! A body is read by [`read_body`]: JSON of the request's shape, else `400 invalid_request` with
//! the JSON pointer of the field it fails at in `errors[0].path` (a wrong type, a missing field,
//! a field the body does not have). A field the body does not have is refused whether its type
//! denies unknown fields or not.
use crate::v2::problem::{FieldError, Problem, ProblemCode};
use axum::extract::{FromRequest, FromRequestParts, Request};
use bytes::Bytes;
use http::StatusCode;
use serde::de::DeserializeOwned;

/// A JSON request body; a merge patch (`application/merge-patch+json`) reads as JSON too.
#[derive(Debug, Clone, Copy, Default)]
pub struct Body<T>(pub T);

impl<T, S> FromRequest<S> for Body<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        if !json_content_type(request.headers()) {
            return Err(Problem::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                ProblemCode::UnsupportedMediaType,
                "Expected request with `Content-Type: application/json`",
            ));
        }
        let bytes = Bytes::from_request(request, state).await?;
        read_body(&bytes).map(Body)
    }
}

/// `application/json`, or a type of it (`application/merge-patch+json`).
fn json_content_type(headers: &http::HeaderMap) -> bool {
    let Some(content_type) = headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match essence.split_once('/') {
        Some(("application", subtype)) => subtype == "json" || subtype.ends_with("+json"),
        _ => false,
    }
}

/// `bytes` read as a `T`, refused at the pointer of the field it fails at.
pub fn read_body<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Problem> {
    let mut json = serde_json::Deserializer::from_slice(bytes);
    let read = read(&mut json)?;
    json.end().map_err(|error| refused(&error, String::new()))?;
    Ok(read)
}

/// A body read as JSON already, refused as [`read_body`] refuses it.
pub fn from_value<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, Problem> {
    read(value)
}

fn read<'de, T, D>(deserializer: D) -> Result<T, Problem>
where
    T: DeserializeOwned,
    D: serde::Deserializer<'de, Error = serde_json::Error>,
{
    let mut unknown: Option<String> = None;
    let mut note = |path: serde_ignored::Path| {
        unknown.get_or_insert_with(|| ignored_pointer(&path));
    };
    let tracked = serde_ignored::Deserializer::new(deserializer, &mut note);
    let read: Result<T, _> = serde_path_to_error::deserialize(tracked);
    match read {
        Ok(read) => match unknown {
            None => Ok(read),
            Some(pointer) => Err(unknown_field(pointer)),
        },
        Err(error) => Err(refused(error.inner(), pointer(error.path()))),
    }
}

/// The JSON pointer of a path `serde_path_to_error` tracked.
fn pointer(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    path.iter()
        .filter_map(|segment| match segment {
            Segment::Seq { index } => Some(index.to_string()),
            Segment::Map { key } => Some(escape(key)),
            Segment::Enum { variant } => Some(escape(variant)),
            Segment::Unknown => None,
        })
        .fold(String::new(), |pointer, token| format!("{pointer}/{token}"))
}

/// The JSON pointer of a key `serde_ignored` saw left unread.
fn ignored_pointer(path: &serde_ignored::Path) -> String {
    use serde_ignored::Path;
    match path {
        Path::Root => String::new(),
        Path::Seq { parent, index } => format!("{}/{index}", ignored_pointer(parent)),
        Path::Map { parent, key } => format!("{}/{}", ignored_pointer(parent), escape(key)),
        Path::Some { parent }
        | Path::NewtypeStruct { parent }
        | Path::NewtypeVariant { parent } => ignored_pointer(parent),
    }
}

/// A token of a JSON pointer (RFC 6901): `~` and `/` escaped.
fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn unknown_field(pointer: String) -> Problem {
    let name = pointer.rsplit('/').next().unwrap_or_default().to_owned();
    let mut problem = Problem::invalid_request(format!("Unknown field `{name}`"));
    problem.errors.push(FieldError {
        path: pointer,
        code: "unknown".to_owned(),
        message: format!("The request has no field `{name}`"),
    });
    problem
}

/// A body that does not read, at `pointer`.
fn refused(error: &serde_json::Error, pointer: String) -> Problem {
    let message = error.to_string();
    // serde_json appends where in the text it stopped: the pointer says it better.
    let message = match message.rfind(" at line ") {
        Some(at) if error.line() > 0 => message[..at].to_owned(),
        _ => message,
    };
    if !error.is_data() {
        return Problem::invalid_request(message);
    }
    // An unknown field is refused at its own key, a missing one at the object missing it.
    let named = |prefix: &str| {
        message
            .strip_prefix(prefix)
            .and_then(|rest| rest.split('`').next())
            .map(|field| {
                let token = format!("/{}", escape(field));
                match pointer.ends_with(&token) {
                    true => pointer.clone(),
                    false => format!("{pointer}{token}"),
                }
            })
    };
    if let Some(field) = named("unknown field `") {
        return unknown_field(field);
    }
    let (path, code) = match named("missing field `") {
        Some(field) => (field, "required"),
        None => (pointer, "invalid"),
    };
    let mut problem = Problem::invalid_request(message.clone());
    problem.errors.push(FieldError {
        path,
        code: code.to_owned(),
        message,
    });
    problem
}

#[derive(Debug, Clone, Copy, Default, FromRequestParts)]
#[from_request(via(axum::extract::Path), rejection(Problem))]
pub struct Path<T>(pub T);

#[derive(Debug, Clone, Copy, Default, FromRequestParts)]
#[from_request(via(axum::extract::Query), rejection(Problem))]
pub struct Query<T>(pub T);

/// A merge patch's field `null` cannot clear (a name, a list a resource needs): refused at
/// `path`; left out, it is unchanged.
pub fn kept<T>(field: Option<Option<T>>, path: &str) -> Result<Option<T>, Problem> {
    match field {
        Some(None) => Err(Problem::field(
            path,
            "required",
            "This field cannot be cleared",
        )),
        Some(Some(value)) => Ok(Some(value)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct Line {
        quantity: u32,
        #[serde(default)]
        note: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    #[serde(deny_unknown_fields)]
    struct Order {
        name: String,
        #[serde(default)]
        lines: Vec<Line>,
    }

    fn refusal(json: &str) -> FieldError {
        let problem = read_body::<Order>(json.as_bytes()).unwrap_err();
        assert_eq!(problem.status, 400);
        problem.errors.into_iter().next().expect("a field refused")
    }

    #[test]
    fn a_body_refused_names_the_field_at_its_pointer() {
        let wrong = refusal(r#"{"name": "a", "lines": [{"quantity": "two"}]}"#);
        assert_eq!(
            (wrong.path.as_str(), wrong.code.as_str()),
            ("/lines/0/quantity", "invalid")
        );
        let missing = refusal(r#"{"name": "a", "lines": [{}]}"#);
        assert_eq!(
            (missing.path.as_str(), missing.code.as_str()),
            ("/lines/0/quantity", "required")
        );
        let root = refusal(r#"{"lines": []}"#);
        assert_eq!(root.path, "/name");
    }

    #[test]
    fn an_unknown_field_is_refused_with_or_without_deny_unknown_fields() {
        let denied = refusal(r#"{"name": "a", "lines": [{"quantity": 1, "qty": 2}]}"#);
        assert_eq!(
            (denied.path.as_str(), denied.code.as_str()),
            ("/lines/0/qty", "unknown")
        );
        let ignored = refusal(r#"{"name": "a", "trial_day": 3}"#);
        assert_eq!(
            (ignored.path.as_str(), ignored.code.as_str()),
            ("/trial_day", "unknown")
        );
        let escaped = refusal(r#"{"name": "a", "a/b": 1}"#);
        assert_eq!(escaped.path, "/a~1b");
    }

    #[test]
    fn a_body_that_is_not_json_is_refused_without_a_field() {
        let problem = read_body::<Order>(b"{\"name\": ").unwrap_err();
        assert_eq!(problem.status, 400);
        assert!(problem.errors.is_empty());
        assert!(read_body::<Order>(br#"{"name": "a"} trailing"#).is_err());
        assert!(read_body::<serde_json::Value>(br#"{"any": {"thing": 1}}"#).is_ok());
    }
}
