//! The credit billing scenarios of `design/scenario-timelines.md` §5, one test per scenario and
//! variant, asserting the timeline's figures exactly and the credit invariants after each step.
//! Other test binaries include this suite: reach the harness through `meteroid_testkit`, not
//! `crate::harness`.
pub(crate) use meteroid_testkit::halyard as kit;

mod s01_scale_to_studio_forced;
mod s02_scale_to_studio_scheduled;
mod s03_studio_to_scale;
mod s04_plan_version_migration;
mod s05_pause_across_expiry;
mod s06_cancel_at_period_end;
mod s07_immediate_termination;
mod s08_trial_then_paid;
mod s09_quote_into_credit_plan;
mod s10_two_subscriptions;
mod s11_product_restricted_grant;
mod s12_promo_code_two_subscriptions;
mod s13_top_up_void_note_refund;
mod s14_overpayment;
mod s15_minimum_commitment;
mod s16_tax_promotional_then_paid;
mod s17_rollover_at_boundaries;
mod s18_credit_units;
mod s19_attribution_key;
mod s20_fiat_credit_on_credit_overage;
mod s21_disputed_payment;
mod s22_provider_refund;
