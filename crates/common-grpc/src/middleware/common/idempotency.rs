pub const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

// header in response of wrapper which signals that
// response was served from the cache
pub const IDEMPOTENCY_CACHE_RESPONSE_HEADER: &str = "x-idempotency-cache-result";
