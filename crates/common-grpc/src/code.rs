use tonic::Code;

pub fn code_as_str(code: tonic::Code) -> &'static str {
    match code {
        Code::Ok => "ok",
        Code::Cancelled => "cancelled",
        Code::Unknown => "unknown",
        Code::InvalidArgument => "invalid-argument",
        Code::DeadlineExceeded => "deadline-exceeded",
        Code::NotFound => "not-found",
        Code::AlreadyExists => "already-exists",
        Code::PermissionDenied => "permission-denied",
        Code::ResourceExhausted => "resource-exhausted",
        Code::FailedPrecondition => "failed-precondition",
        Code::Aborted => "aborted",
        Code::OutOfRange => "out-of-range",
        Code::Unimplemented => "unimplemented",
        Code::Internal => "internal-error",
        Code::Unavailable => "unavailable",
        Code::DataLoss => "data-loss",
        Code::Unauthenticated => "unauthenticated",
    }
}
