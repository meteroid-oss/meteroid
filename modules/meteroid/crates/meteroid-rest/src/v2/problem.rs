//! Errors of API v2: RFC 9457 problem details, `application/problem+json`, each with a specific
//! `code` and, for a refused field, a JSON pointer to it (ADR-0013 §3).
use axum::extract::rejection::{BytesRejection, JsonRejection, PathRejection, QueryRejection};
use axum::response::{IntoResponse, Response};
use error_stack::Report;
use http::{HeaderValue, StatusCode};
use meteroid_app::errors::RestApiError;
use meteroid_store::errors::{IdempotencyKeyReused, NotInPlan, RateLimited, StoreError};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";
const TYPE_BASE: &str = "https://docs.meteroid.com/api/v2/errors/";

/// What went wrong, specific enough to act on. New codes may be added without a new
/// `Meteroid-Version`: a client reads one it does not know by its HTTP status.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProblemCode {
    /// The request could not be read: malformed JSON, a wrong type, an unknown parameter value.
    InvalidRequest,
    /// The request was read but a field is refused: see `errors`.
    ValidationFailed,
    /// The model cannot express what was asked (a flat price on seats, arrears seats…).
    NotExpressible,
    NotFound,
    /// The code is used by another resource of the same kind.
    CodeTaken,
    AlreadyExists,
    /// The resource's state refuses the change.
    Conflict,
    /// The plan has no draft.
    NoDraft,
    /// The change cannot take effect when asked (`when`): an immediate downgrade, a date where
    /// only now or the period's end are possible…
    WhenRefused,
    /// The `Idempotency-Key` was used with another request.
    IdempotencyKeyReused,
    /// The first request with this `Idempotency-Key` is still running.
    IdempotencyRequestInProgress,
    /// The operation moves money and is only sent with an `Idempotency-Key`.
    IdempotencyKeyRequired,
    /// `Meteroid-Version` is not a date on or after the first version.
    UnsupportedVersion,
    Unauthorized,
    /// The client API's customer token is genuine but expired: mint another.
    TokenExpired,
    /// The credentials may not do this: a scope, delegated access, an express organization.
    Forbidden,
    /// The organization's plan or edition does not include this feature, or it is not turned on
    /// for the organization.
    FeatureNotAvailable,
    RateLimited,
    PayloadTooLarge,
    MethodNotAllowed,
    UnsupportedMediaType,
    Unavailable,
    NotImplemented,
    InternalError,
}

impl ProblemCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ProblemCode::InvalidRequest => "invalid_request",
            ProblemCode::ValidationFailed => "validation_failed",
            ProblemCode::NotExpressible => "not_expressible",
            ProblemCode::NotFound => "not_found",
            ProblemCode::CodeTaken => "code_taken",
            ProblemCode::AlreadyExists => "already_exists",
            ProblemCode::Conflict => "conflict",
            ProblemCode::NoDraft => "no_draft",
            ProblemCode::WhenRefused => "when_refused",
            ProblemCode::IdempotencyKeyReused => "idempotency_key_reused",
            ProblemCode::IdempotencyRequestInProgress => "idempotency_request_in_progress",
            ProblemCode::IdempotencyKeyRequired => "idempotency_key_required",
            ProblemCode::UnsupportedVersion => "unsupported_version",
            ProblemCode::Unauthorized => "unauthorized",
            ProblemCode::TokenExpired => "token_expired",
            ProblemCode::Forbidden => "forbidden",
            ProblemCode::FeatureNotAvailable => "feature_not_available",
            ProblemCode::RateLimited => "rate_limited",
            ProblemCode::PayloadTooLarge => "payload_too_large",
            ProblemCode::MethodNotAllowed => "method_not_allowed",
            ProblemCode::UnsupportedMediaType => "unsupported_media_type",
            ProblemCode::Unavailable => "unavailable",
            ProblemCode::NotImplemented => "not_implemented",
            ProblemCode::InternalError => "internal_error",
        }
    }

    fn title(self) -> &'static str {
        match self {
            ProblemCode::InvalidRequest => "The request could not be read",
            ProblemCode::ValidationFailed => "A field of the request is refused",
            ProblemCode::NotExpressible => "The model cannot express this",
            ProblemCode::NotFound => "Not found",
            ProblemCode::CodeTaken => "The code is taken",
            ProblemCode::AlreadyExists => "It already exists",
            ProblemCode::Conflict => "The resource's state refuses the change",
            ProblemCode::NoDraft => "The plan has no draft",
            ProblemCode::WhenRefused => "The change cannot take effect when asked",
            ProblemCode::IdempotencyKeyReused => {
                "The Idempotency-Key was used with another request"
            }
            ProblemCode::IdempotencyRequestInProgress => {
                "The first request with this Idempotency-Key is still running"
            }
            ProblemCode::IdempotencyKeyRequired => "An Idempotency-Key is required",
            ProblemCode::UnsupportedVersion => "Unsupported Meteroid-Version",
            ProblemCode::Unauthorized => "Unauthorized",
            ProblemCode::TokenExpired => "The token expired",
            ProblemCode::Forbidden => "Forbidden",
            ProblemCode::FeatureNotAvailable => "The feature is not available",
            ProblemCode::RateLimited => "Too many requests",
            ProblemCode::PayloadTooLarge => "The request body is too large",
            ProblemCode::MethodNotAllowed => "Method not allowed",
            ProblemCode::UnsupportedMediaType => "Unsupported media type",
            ProblemCode::Unavailable => "Unavailable, retry later",
            ProblemCode::NotImplemented => "Not implemented",
            ProblemCode::InternalError => "Internal error",
        }
    }

    /// The code an error status answered outside a handler reads as.
    pub fn of_status(status: StatusCode) -> Self {
        match status {
            StatusCode::BAD_REQUEST => ProblemCode::InvalidRequest,
            StatusCode::UNAUTHORIZED => ProblemCode::Unauthorized,
            StatusCode::FORBIDDEN => ProblemCode::Forbidden,
            StatusCode::NOT_FOUND => ProblemCode::NotFound,
            StatusCode::METHOD_NOT_ALLOWED => ProblemCode::MethodNotAllowed,
            StatusCode::CONFLICT => ProblemCode::Conflict,
            StatusCode::PAYLOAD_TOO_LARGE => ProblemCode::PayloadTooLarge,
            StatusCode::UNSUPPORTED_MEDIA_TYPE => ProblemCode::UnsupportedMediaType,
            StatusCode::UNPROCESSABLE_ENTITY => ProblemCode::ValidationFailed,
            StatusCode::TOO_MANY_REQUESTS => ProblemCode::RateLimited,
            StatusCode::NOT_IMPLEMENTED => ProblemCode::NotImplemented,
            StatusCode::SERVICE_UNAVAILABLE | StatusCode::BAD_GATEWAY => ProblemCode::Unavailable,
            status if status.is_client_error() => ProblemCode::InvalidRequest,
            _ => ProblemCode::InternalError,
        }
    }
}

/// Why a field is refused. New codes may be added without a new `Meteroid-Version`: a client
/// reads one it does not know as `invalid`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldErrorCode {
    /// Left out, or empty, where it is required (a header too: `Idempotency-Key`).
    Required,
    /// Present but not a value it can take.
    Invalid,
    /// A field the body does not take (a misspelt name).
    Unknown,
    /// It names something that does not exist, or not for this tenant.
    NotFound,
    /// It names something archived, which takes no new use.
    Archived,
    /// It could name more than one thing (a product several charges bill).
    Ambiguous,
    /// Its value is another resource's, or repeated in the request.
    Duplicate,
    /// The model cannot express it (`not_expressible`).
    NotExpressible,
    /// It contradicts the preset the charge names.
    PresetConflict,
    /// The change cannot take effect when asked (`when_refused`).
    WhenRefused,
}

impl FieldErrorCode {
    pub const ALL: [FieldErrorCode; 10] = [
        FieldErrorCode::Required,
        FieldErrorCode::Invalid,
        FieldErrorCode::Unknown,
        FieldErrorCode::NotFound,
        FieldErrorCode::Archived,
        FieldErrorCode::Ambiguous,
        FieldErrorCode::Duplicate,
        FieldErrorCode::NotExpressible,
        FieldErrorCode::PresetConflict,
        FieldErrorCode::WhenRefused,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            FieldErrorCode::Required => "required",
            FieldErrorCode::Invalid => "invalid",
            FieldErrorCode::Unknown => "unknown",
            FieldErrorCode::NotFound => "not_found",
            FieldErrorCode::Archived => "archived",
            FieldErrorCode::Ambiguous => "ambiguous",
            FieldErrorCode::Duplicate => "duplicate",
            FieldErrorCode::NotExpressible => "not_expressible",
            FieldErrorCode::PresetConflict => "preset_conflict",
            FieldErrorCode::WhenRefused => "when_refused",
        }
    }

    /// Whether `code` is one of the documented codes.
    pub fn documents(code: &str) -> bool {
        Self::ALL.iter().any(|known| known.as_str() == code)
    }
}

/// A field the request was refused for.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    /// JSON pointer into the request body (`/charges/0/prices/1/model`), or the parameter or
    /// header name (`limit`, `Idempotency-Key`).
    pub path: String,
    /// Why the field is refused.
    // Text, so a problem read back (the envelope, a replay) keeps its code whatever it is; every
    // code answered is a `FieldErrorCode` (asserted in debug builds by `into_response`).
    #[schema(value_type = FieldErrorCode)]
    pub code: String,
    pub message: String,
}

/// An RFC 9457 problem: every v2 error answers one, as `application/problem+json`.
#[derive(Serialize, Deserialize, ToSchema, Debug, Clone)]
pub struct Problem {
    /// A URI naming the kind of problem, documented per `code`.
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub status: u16,
    pub code: ProblemCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldError>,
    /// The id of the request, as in the `Request-Id` header: quote it to support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl Problem {
    pub fn new(status: StatusCode, code: ProblemCode, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Problem {
            kind: format!("{TYPE_BASE}{}", code.as_str()),
            title: code.title().to_owned(),
            status: status.as_u16(),
            code,
            detail: (!detail.is_empty()).then_some(detail),
            errors: Vec::new(),
            request_id: None,
        }
    }

    pub fn status(&self) -> StatusCode {
        StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    }

    pub fn invalid_request(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ProblemCode::InvalidRequest, detail)
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ProblemCode::NotFound, detail)
    }

    pub fn conflict(code: ProblemCode, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, detail)
    }

    /// A money write sent without an `Idempotency-Key`: `400 idempotency_key_required`, the
    /// header named in `errors`.
    pub fn key_required(message: impl Into<String>) -> Self {
        let message = message.into();
        let mut problem = Self::new(
            StatusCode::BAD_REQUEST,
            ProblemCode::IdempotencyKeyRequired,
            message.clone(),
        );
        problem.errors.push(FieldError {
            path: crate::v2::idempotency::IDEMPOTENCY_KEY_HEADER.to_owned(),
            code: FieldErrorCode::Required.as_str().to_owned(),
            message,
        });
        problem
    }

    /// The organization's plan or edition does not include what was asked, or it is not turned
    /// on for the organization: `403 feature_not_available`.
    pub fn feature_not_available(detail: impl Into<String>) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            ProblemCode::FeatureNotAvailable,
            detail,
        )
    }

    /// A gate's refusal (`meteroid_app::features`): a feature the plan lacks, its message the
    /// detail; any other status as v1 reads it.
    pub fn gated(status: tonic::Status) -> Self {
        if status.code() == tonic::Code::PermissionDenied {
            Self::feature_not_available(status.message())
        } else {
            Self::from(RestApiError::from(status))
        }
    }

    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ProblemCode::InternalError,
            "Internal error. Quote the request_id to support.",
        )
    }

    /// One field refused: `validation_failed` naming it.
    pub fn field(path: impl Into<String>, code: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        let mut problem = Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
            message.clone(),
        );
        problem.errors.push(FieldError {
            path: path.into(),
            code: code.to_owned(),
            message,
        });
        problem
    }

    /// The `validator` rules a body breaks, each at its JSON pointer (`/events/0/code`).
    pub fn invalid_fields(errors: &validator::ValidationErrors) -> Self {
        fn collect(errors: &validator::ValidationErrors, prefix: &str, out: &mut Vec<FieldError>) {
            for (field, kind) in errors.errors() {
                let path = format!("{prefix}/{field}");
                match kind {
                    validator::ValidationErrorsKind::Field(refusals) => {
                        out.extend(refusals.iter().map(|refusal| {
                            FieldError {
                                path: path.clone(),
                                code: "invalid".to_owned(),
                                message: refusal
                                    .message
                                    .as_ref()
                                    .map(|message| message.to_string())
                                    .unwrap_or_else(|| refusal.code.to_string()),
                            }
                        }))
                    }
                    validator::ValidationErrorsKind::Struct(inner) => collect(inner, &path, out),
                    validator::ValidationErrorsKind::List(items) => {
                        for (index, inner) in items {
                            collect(inner, &format!("{path}/{index}"), out);
                        }
                    }
                }
            }
        }
        let mut problem = Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
            errors.to_string(),
        );
        collect(errors, "", &mut problem.errors);
        problem.errors.sort_by(|a, b| a.path.cmp(&b.path));
        problem
    }

    /// What the model cannot express, at `path`: refused, never stored differently.
    pub fn not_expressible(path: impl Into<String>, message: impl Into<String>) -> Self {
        let mut problem = Self::field(path, "not_expressible", message);
        problem.code = ProblemCode::NotExpressible;
        problem.kind = format!("{TYPE_BASE}{}", ProblemCode::NotExpressible.as_str());
        problem.title = ProblemCode::NotExpressible.title().to_owned();
        problem
    }

    /// A change refused at the moment `when` asks, at `path` (`/when/type`, `/when/date`).
    pub fn when_refused(path: impl Into<String>, message: impl Into<String>) -> Self {
        let mut problem = Self::field(path, "when_refused", message);
        problem.code = ProblemCode::WhenRefused;
        problem.kind = format!("{TYPE_BASE}{}", ProblemCode::WhenRefused.as_str());
        problem.title = ProblemCode::WhenRefused.title().to_owned();
        problem
    }

    /// A store refusal, its message as the detail of a refused field at `path`.
    pub fn at(path: impl Into<String>, error: Report<StoreError>) -> Self {
        let mut problem = Problem::from(error);
        if problem.code == ProblemCode::ValidationFailed && problem.errors.is_empty() {
            problem.errors.push(FieldError {
                path: path.into(),
                code: "invalid".to_owned(),
                message: problem.detail.clone().unwrap_or_default(),
            });
        }
        problem
    }

    /// A unique violation on create read as the code being taken.
    pub fn code_taken(self, code: &str) -> Self {
        if self.code == ProblemCode::AlreadyExists {
            let mut taken = Self::conflict(
                ProblemCode::CodeTaken,
                format!("The code '{code}' is taken"),
            );
            taken.errors.push(FieldError {
                path: "/code".to_owned(),
                code: "duplicate".to_owned(),
                message: format!("The code '{code}' is taken"),
            });
            return taken;
        }
        self
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = self.status();
        debug_assert!(
            self.errors
                .iter()
                .all(|error| FieldErrorCode::documents(&error.code)),
            "a field error code missing from FieldErrorCode: {self:?}"
        );
        if status.is_server_error() {
            log::error!("Responding with HTTP {status}: {self:?}");
        }
        let body = serde_json::to_vec(&self).unwrap_or_default();
        let mut response = (status, body).into_response();
        response.headers_mut().insert(
            http::header::CONTENT_TYPE,
            HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
        );
        response
    }
}

pub fn store_message(error: &StoreError) -> String {
    match error {
        StoreError::InvalidArgument(message)
        | StoreError::Conflict(message)
        | StoreError::FailedPrecondition(message)
        | StoreError::Forbidden(message)
        | StoreError::ValueNotFound(message) => message.clone(),
        other => other.to_string(),
    }
}

impl From<Report<StoreError>> for Problem {
    fn from(error: Report<StoreError>) -> Self {
        if let Some(RateLimited(message)) = error
            .frames()
            .find_map(|frame| frame.downcast_ref::<RateLimited>())
        {
            return Problem::new(
                StatusCode::TOO_MANY_REQUESTS,
                ProblemCode::RateLimited,
                message.clone(),
            );
        }
        // The store's own claim of the key (a write keyed beside the middleware's) refusing it.
        if error
            .frames()
            .any(|frame| frame.downcast_ref::<IdempotencyKeyReused>().is_some())
        {
            return Problem::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemCode::IdempotencyKeyReused,
                "This Idempotency-Key was used with another request: use a new key for a new \
                 request",
            );
        }
        let context = error.current_context();
        match context {
            StoreError::ValueNotFound(_) => Problem::not_found(store_message(context)),
            // A row a query was sure to find: the resource asked for does not exist.
            StoreError::DatabaseError(_) if context.is_row_not_found() => {
                Problem::not_found("Not found")
            }
            StoreError::DuplicateValue { .. } => {
                Problem::conflict(ProblemCode::AlreadyExists, "It already exists")
            }
            StoreError::Conflict(_) | StoreError::FailedPrecondition(_) => {
                Problem::conflict(ProblemCode::Conflict, store_message(context))
            }
            StoreError::InvalidArgument(_) | StoreError::InvalidPriceComponents(_) => Problem::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemCode::ValidationFailed,
                store_message(context),
            ),
            StoreError::Forbidden(_)
                if error
                    .frames()
                    .any(|frame| frame.downcast_ref::<NotInPlan>().is_some()) =>
            {
                Problem::feature_not_available(store_message(context))
            }
            StoreError::Forbidden(_) => Problem::new(
                StatusCode::FORBIDDEN,
                ProblemCode::Forbidden,
                store_message(context),
            ),
            StoreError::UsageUnavailable | StoreError::MeteringNotReady => Problem::new(
                StatusCode::SERVICE_UNAVAILABLE,
                ProblemCode::Unavailable,
                context.to_string(),
            ),
            // Metering down or late reads as v1 reads it: `503 unavailable`, else internal.
            StoreError::MeteringServiceError => Problem::from(RestApiError::from(error)),
            _ => {
                log::error!("Store error while handling a v2 request: {error:?}");
                Problem::internal()
            }
        }
    }
}

impl From<RestApiError> for Problem {
    fn from(error: RestApiError) -> Self {
        match error {
            RestApiError::InvalidInput(message) => Problem::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemCode::ValidationFailed,
                message,
            ),
            RestApiError::NotFound => Problem::not_found(""),
            RestApiError::Conflict => Problem::conflict(ProblemCode::Conflict, ""),
            RestApiError::FailedPrecondition(message) => {
                Problem::conflict(ProblemCode::Conflict, message)
            }
            RestApiError::Unauthorized => {
                Problem::new(StatusCode::UNAUTHORIZED, ProblemCode::Unauthorized, "")
            }
            // v1 refuses a feature the plan lacks with it, and nothing else v2 reaches.
            RestApiError::Forbidden => {
                Problem::feature_not_available("Your plan does not include this feature")
            }
            RestApiError::RateLimitExceeded(message) => Problem::new(
                StatusCode::TOO_MANY_REQUESTS,
                ProblemCode::RateLimited,
                message,
            ),
            RestApiError::Unavailable(message) | RestApiError::NotConfigured(message) => {
                Problem::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ProblemCode::Unavailable,
                    message,
                )
            }
            RestApiError::NotImplemented(message) => Problem::new(
                StatusCode::NOT_IMPLEMENTED,
                ProblemCode::NotImplemented,
                message,
            ),
            other => {
                log::error!("Error while handling a v2 request: {other:?}");
                Problem::internal()
            }
        }
    }
}

macro_rules! from_rejection {
    ($($rejection:ty),+) => {$(
        impl From<$rejection> for Problem {
            fn from(rejection: $rejection) -> Self {
                let status = rejection.status();
                let code = match status {
                    StatusCode::UNPROCESSABLE_ENTITY => ProblemCode::InvalidRequest,
                    other => ProblemCode::of_status(other),
                };
                // A body that does not deserialize is unreadable (400), not a refused field.
                let status = if status == StatusCode::UNPROCESSABLE_ENTITY {
                    StatusCode::BAD_REQUEST
                } else {
                    status
                };
                Problem::new(status, code, rejection.body_text())
            }
        }
    )+};
}
from_rejection!(JsonRejection, PathRejection, BytesRejection);

/// A query string that does not deserialize is a refused parameter, as a `limit` or a cursor out
/// of range is: `422 validation_failed` naming it (`status`, `granularity`, `from`) when the
/// refusal says which; the parameter's own message otherwise.
impl From<QueryRejection> for Problem {
    fn from(rejection: QueryRejection) -> Self {
        let text = rejection.body_text();
        let refusal = text
            .strip_prefix("Failed to deserialize query string: ")
            .unwrap_or(text.as_str());
        let mut problem = Problem::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemCode::ValidationFailed,
            refusal,
        );
        if let Some((parameter, message)) = query_refusal(refusal) {
            problem.errors.push(FieldError {
                path: parameter,
                code: "invalid".to_owned(),
                message,
            });
        }
        problem
    }
}

/// The parameter a query refusal names and what is wrong with it: `status: unknown variant …`
/// (its path, as `serde_path_to_error` writes it), or `missing field `from``.
fn query_refusal(refusal: &str) -> Option<(String, String)> {
    if let Some((parameter, message)) = refusal.split_once(": ")
        && !parameter.is_empty()
        && parameter
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '[' | ']'))
    {
        return Some((parameter.to_owned(), message.to_owned()));
    }
    let missing = refusal.strip_prefix("missing field `")?.strip_suffix('`')?;
    Some((missing.to_owned(), format!("{missing} is required")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_serializes_as_rfc_9457() {
        let mut problem = Problem::field("/charges/0/prices", "required", "a price is required");
        problem.request_id = Some("req_1".to_owned());
        let json = serde_json::to_value(&problem).unwrap();
        assert_eq!(
            json["type"],
            "https://docs.meteroid.com/api/v2/errors/validation_failed"
        );
        assert_eq!(json["status"], 422);
        assert_eq!(json["code"], "validation_failed");
        assert_eq!(json["errors"][0]["path"], "/charges/0/prices");
        assert_eq!(json["request_id"], "req_1");
    }

    #[test]
    fn a_query_refusal_names_its_parameter() {
        assert_eq!(
            query_refusal("status: unknown variant `CREATED`, expected `created` or `expired`"),
            Some((
                "status".to_owned(),
                "unknown variant `CREATED`, expected `created` or `expired`".to_owned()
            ))
        );
        assert_eq!(
            query_refusal("missing field `from`"),
            Some(("from".to_owned(), "from is required".to_owned()))
        );
        assert_eq!(query_refusal("invalid type: expected a map"), None);
    }

    #[test]
    fn a_store_refusal_reads_as_its_problem() {
        let not_found: Problem =
            Report::new(StoreError::ValueNotFound("plan pro".to_owned())).into();
        assert_eq!(not_found.code, ProblemCode::NotFound);
        assert_eq!(not_found.status, 404);
        let duplicate: Problem = Report::new(StoreError::DuplicateValue {
            entity: "db entity",
            key: None,
        })
        .into();
        assert_eq!(duplicate.code_taken("pro").code, ProblemCode::CodeTaken);
    }

    #[test]
    fn a_feature_the_plan_lacks_is_told_apart_from_authorization() {
        let lacking: Problem = Report::new(StoreError::Forbidden("not in plan".to_owned()))
            .attach_opaque(NotInPlan)
            .into();
        assert_eq!(lacking.code, ProblemCode::FeatureNotAvailable);
        assert_eq!(lacking.status, 403);
        let refused: Problem = Report::new(StoreError::Forbidden("not yours".to_owned())).into();
        assert_eq!(refused.code, ProblemCode::Forbidden);
        assert_eq!(
            Problem::from(RestApiError::Forbidden).code,
            ProblemCode::FeatureNotAvailable
        );
    }

    #[test]
    fn a_money_write_without_a_key_names_the_header() {
        let json = serde_json::to_value(Problem::key_required("a key")).unwrap();
        assert_eq!(json["status"], 400);
        assert_eq!(json["code"], "idempotency_key_required");
        assert_eq!(json["errors"][0]["path"], "Idempotency-Key");
        assert_eq!(json["errors"][0]["code"], "required");
    }

    #[test]
    fn every_field_error_code_reads_as_documented() {
        for code in FieldErrorCode::ALL {
            assert_eq!(
                serde_json::to_value(code).unwrap(),
                serde_json::json!(code.as_str())
            );
        }
    }

    #[test]
    fn a_key_the_store_refuses_for_another_request_reads_as_reused() {
        let reused: Problem = Report::new(StoreError::Conflict(
            "this Idempotency-Key was already used with a different request".to_owned(),
        ))
        .attach_opaque(IdempotencyKeyReused)
        .into();
        assert_eq!(reused.code, ProblemCode::IdempotencyKeyReused);
        assert_eq!(reused.status, 422);
        let conflict: Problem = Report::new(StoreError::Conflict("taken".to_owned())).into();
        assert_eq!(conflict.code, ProblemCode::Conflict);
    }
}
