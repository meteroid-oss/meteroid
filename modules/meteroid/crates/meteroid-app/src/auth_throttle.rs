//! Auth-flow rate limiting (per-IP and per-email/per-user) for the
//! unauthenticated gRPC endpoints — Login, VerifyMfa, InitResetPassword,
//! InitRegistration, CompleteRegistration.
//!
//! ## Two layers, both backed by the same token-bucket limiter
//!
//! * **Per-IP** is enforced via the existing `RateLimitLayer` in the tower
//!   stack (see `api/server.rs`). Catches naive single-source brute force.
//!
//! * **Per-email / per-user** is enforced in-handler by calling
//!   [`AuthThrottle`] before the store call. Catches *distributed*
//!   credential stuffing where the attacker rotates IPs but hammers the
//!   same address. Keys are SHA256-truncated so raw emails never land in
//!   Redis logs.
//!
//! Both layers use the shared `rate_limiter::RateLimiter`, whose backend (when the edition has
//! one) is shared across replicas. Without a backend, or when it fails, the limiter allows the
//! request — same behavior as the rest of the codebase.

use meteroid_store::domain::org_security_audit_log::SecurityRequestContext;
use rate_limiter::{RateLimitConfig, RateLimitResult, RateLimiter};
use sha2::{Digest, Sha256};
use tonic::Status;

/// Pre-configured per-bucket limiters for the auth endpoints. Cloning is
/// cheap — each limiter holds an `Arc` to the shared backend.
#[derive(Clone)]
pub struct AuthThrottle {
    login_ip: RateLimiter,
    login_email: RateLimiter,
    verify_mfa_ip: RateLimiter,
    verify_mfa_user: RateLimiter,
    init_reset_ip: RateLimiter,
    init_reset_email: RateLimiter,
    init_reg_ip: RateLimiter,
    init_reg_email: RateLimiter,
    complete_reg_ip: RateLimiter,
    complete_reg_email: RateLimiter,
}

impl AuthThrottle {
    /// Build the throttle from the shared base limiter, attaching a config
    /// per endpoint. Numbers are SOC2-compliant defaults; tune via
    /// configuration in a follow-up if production needs differ.
    ///
    /// Token-bucket semantics: `capacity` is the burst tolerance, the bucket
    /// refills at `refill_rate` tokens/sec, and Redis keys expire after
    /// `window_seconds` of inactivity.
    pub fn from_base(base: &RateLimiter) -> Self {
        Self {
            login_ip: base.with_config(bucket(
                "auth:login:ip",
                /* capacity (burst) */ 30.0,
                /* sustained */ 100.0,
                /* window */ 900,
            )),
            login_email: base.with_config(bucket("auth:login:email", 5.0, 20.0, 900)),
            verify_mfa_ip: base.with_config(bucket("auth:verify_mfa:ip", 30.0, 100.0, 900)),
            verify_mfa_user: base.with_config(bucket("auth:verify_mfa:user", 5.0, 20.0, 900)),
            init_reset_ip: base.with_config(bucket("auth:reset:ip", 3.0, 5.0, 3600)),
            init_reset_email: base.with_config(bucket("auth:reset:email", 2.0, 5.0, 3600)),
            init_reg_ip: base.with_config(bucket("auth:register_init:ip", 3.0, 10.0, 3600)),
            init_reg_email: base.with_config(bucket("auth:register_init:email", 2.0, 5.0, 3600)),
            complete_reg_ip: base.with_config(bucket("auth:register_complete:ip", 3.0, 3.0, 60)),
            complete_reg_email: base.with_config(bucket(
                "auth:register_complete:email",
                5.0,
                20.0,
                900,
            )),
        }
    }

    pub async fn check_login(
        &self,
        ctx: &SecurityRequestContext,
        email: &str,
    ) -> Result<(), Status> {
        check_pair("login", &self.login_ip, &self.login_email, ctx, Some(email)).await
    }

    pub async fn check_verify_mfa(
        &self,
        ctx: &SecurityRequestContext,
        user_id_for_key: &str,
    ) -> Result<(), Status> {
        check_pair(
            "verify_mfa",
            &self.verify_mfa_ip,
            &self.verify_mfa_user,
            ctx,
            Some(user_id_for_key),
        )
        .await
    }

    pub async fn check_init_reset_password(
        &self,
        ctx: &SecurityRequestContext,
        email: &str,
    ) -> Result<(), Status> {
        check_pair(
            "init_reset_password",
            &self.init_reset_ip,
            &self.init_reset_email,
            ctx,
            Some(email),
        )
        .await
    }

    pub async fn check_init_registration(
        &self,
        ctx: &SecurityRequestContext,
        email: &str,
    ) -> Result<(), Status> {
        check_pair(
            "init_registration",
            &self.init_reg_ip,
            &self.init_reg_email,
            ctx,
            Some(email),
        )
        .await
    }

    pub async fn check_complete_registration(
        &self,
        ctx: &SecurityRequestContext,
        email: &str,
    ) -> Result<(), Status> {
        check_pair(
            "complete_registration",
            &self.complete_reg_ip,
            &self.complete_reg_email,
            ctx,
            Some(email),
        )
        .await
    }
}

fn bucket(
    name: &str,
    capacity: f64,
    sustained_per_window: f64,
    window_seconds: u64,
) -> RateLimitConfig {
    RateLimitConfig {
        capacity,
        refill_rate: sustained_per_window / window_seconds as f64,
        window_seconds,
        bucket: Some(name.to_string()),
    }
}

async fn check_pair(
    flow: &'static str,
    ip_limiter: &RateLimiter,
    secondary_limiter: &RateLimiter,
    ctx: &SecurityRequestContext,
    secondary_key: Option<&str>,
) -> Result<(), Status> {
    // Per-IP first. If no IP can be extracted (test harness, etc.) skip
    // silently — the per-email layer still catches it.
    if let Some(ip) = ctx.ip_address.as_deref() {
        match ip_limiter.check(ip).await {
            RateLimitResult::Allowed { .. } => {}
            RateLimitResult::RateLimited {
                retry_after, limit, ..
            } => {
                tracing::warn!(
                    flow = flow,
                    scope = "ip",
                    ip = ip,
                    limit = limit,
                    retry_after_seconds = retry_after,
                    "auth_throttle: rate-limit hit",
                );
                return Err(rate_limited_status(retry_after));
            }
        }
    }

    if let Some(key) = secondary_key {
        let hashed = hash_secondary_key(key);
        match secondary_limiter.check(&hashed).await {
            RateLimitResult::Allowed { .. } => {}
            RateLimitResult::RateLimited {
                retry_after, limit, ..
            } => {
                tracing::warn!(
                    flow = flow,
                    scope = "secondary",
                    // Truncated hash — distinguishes targets in the log
                    // without putting raw emails / user-ids in plaintext.
                    secondary_key_hash = %hashed,
                    limit = limit,
                    retry_after_seconds = retry_after,
                    "auth_throttle: rate-limit hit",
                );
                return Err(rate_limited_status(retry_after));
            }
        }
    }

    Ok(())
}

fn rate_limited_status(retry_after: u64) -> Status {
    let mut s = Status::resource_exhausted(format!(
        "Too many requests. Retry in {retry_after} seconds."
    ));
    // Honor RFC 6585: surface Retry-After so clients can back off correctly.
    if let Ok(value) = retry_after.to_string().parse() {
        s.metadata_mut().insert("retry-after", value);
    }
    s
}

/// Hash an identifier (email or user id) to a 16-char hex string. Keeps
/// PII out of Redis keys and SRE logs while remaining stable for joins.
fn hash_secondary_key(s: &str) -> String {
    let normalized = s.trim().to_lowercase();
    let mut h = Sha256::new();
    h.update(normalized.as_bytes());
    let digest = h.finalize();
    let mut hex = String::with_capacity(16);
    for byte in &digest[..8] {
        hex.push_str(&format!("{:02x}", byte));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_case_insensitive() {
        let a = hash_secondary_key("Alice@Example.com");
        let b = hash_secondary_key("  alice@example.com  ");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn different_emails_produce_different_hashes() {
        assert_ne!(
            hash_secondary_key("alice@example.com"),
            hash_secondary_key("bob@example.com"),
        );
    }
}
