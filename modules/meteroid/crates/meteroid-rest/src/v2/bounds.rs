//! The bounds of v2 inputs (ADR-0013 §3), each refused at its JSON pointer with
//! `validation_failed`: what the store would wrap, overflow or let grow without limit is refused
//! before it, with room to spare for any real catalog.
use crate::v2::problem::Problem;

/// A name, in characters: 1 to 255 (blank names are refused where each is read).
pub const MAX_NAME_CHARS: usize = 255;
/// A description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 4096;
/// Net terms, in days.
pub const MAX_NET_TERMS_DAYS: u32 = 365;
/// A trial, in days (ten years).
pub const MAX_TRIAL_DAYS: u32 = 3650;
/// A checkout session's expiry, in hours (a year); 0 never expires.
pub const MAX_CHECKOUT_EXPIRY_HOURS: u32 = 8760;
/// The day of the month periods start on: a shorter month starts them on its last day.
pub const BILLING_DAY_ANCHORS: std::ops::RangeInclusive<u16> = 1..=31;
/// The most charges a plan version holds.
pub const MAX_CHARGES: usize = 100;
/// The most options a plan version sells.
pub const MAX_OPTIONS: usize = 20;
/// The most add-ons a plan version offers, or a purchase holds.
pub const MAX_ADD_ONS: usize = 100;
/// The most grants an entitlements write holds.
pub const MAX_ENTITLEMENTS: usize = 500;
/// The most coupons a purchase redeems.
pub const MAX_COUPONS: usize = 20;
/// The most rates (set or removed) one rate card write holds.
pub const MAX_RATES_PER_WRITE: usize = 5000;

/// Refuses at `path` a list of `len` items longer than `max`.
pub fn count(len: usize, max: usize, path: &str, what: &str) -> Result<(), Problem> {
    if len > max {
        return Err(Problem::field(
            path,
            "invalid",
            format!("{max} {what} at most"),
        ));
    }
    Ok(())
}

/// Refuses at `path` a name longer than [`MAX_NAME_CHARS`].
pub fn name(name: &str, path: &str) -> Result<(), Problem> {
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(Problem::field(
            path,
            "invalid",
            format!("A name is {MAX_NAME_CHARS} characters at most"),
        ));
    }
    Ok(())
}

/// Refuses at `path` a description longer than [`MAX_DESCRIPTION_CHARS`].
pub fn description(description: Option<&str>, path: &str) -> Result<(), Problem> {
    if description.is_some_and(|text| text.chars().count() > MAX_DESCRIPTION_CHARS) {
        return Err(Problem::field(
            path,
            "invalid",
            format!("A description is {MAX_DESCRIPTION_CHARS} characters at most"),
        ));
    }
    Ok(())
}

/// Refuses at `path` net terms past [`MAX_NET_TERMS_DAYS`].
pub fn net_terms(days: u32, path: &str) -> Result<(), Problem> {
    if days > MAX_NET_TERMS_DAYS {
        return Err(Problem::field(
            path,
            "invalid",
            format!("Net terms are 0 to {MAX_NET_TERMS_DAYS} days"),
        ));
    }
    Ok(())
}

/// Refuses at `path` a trial past [`MAX_TRIAL_DAYS`].
pub fn trial_days(days: u32, path: &str) -> Result<(), Problem> {
    if days > MAX_TRIAL_DAYS {
        return Err(Problem::field(
            path,
            "invalid",
            format!("A trial is 0 to {MAX_TRIAL_DAYS} days"),
        ));
    }
    Ok(())
}

/// Refuses at `path` a day of the month outside [`BILLING_DAY_ANCHORS`].
pub fn billing_day_anchor(day: u16, path: &str) -> Result<(), Problem> {
    if !BILLING_DAY_ANCHORS.contains(&day) {
        return Err(Problem::field(
            path,
            "invalid",
            "A day of the month, 1 to 31: a shorter month starts its period on its last day",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_refuse_at_their_pointer() {
        assert!(count(100, 100, "/charges", "charges").is_ok());
        assert_eq!(
            count(101, 100, "/charges", "charges").unwrap_err().errors[0].path,
            "/charges"
        );
        assert!(name(&"é".repeat(255), "/name").is_ok());
        assert!(name(&"é".repeat(256), "/name").is_err());
        assert!(description(Some(&"x".repeat(4097)), "/description").is_err());
        assert!(description(None, "/description").is_ok());
        assert!(net_terms(365, "/net_terms").is_ok());
        assert!(net_terms(366, "/net_terms").is_err());
        assert!(trial_days(3651, "/trial_days").is_err());
        assert!(billing_day_anchor(0, "/billing_day_anchor").is_err());
        assert!(billing_day_anchor(31, "/billing_day_anchor").is_ok());
        assert!(billing_day_anchor(32, "/billing_day_anchor").is_err());
    }
}
