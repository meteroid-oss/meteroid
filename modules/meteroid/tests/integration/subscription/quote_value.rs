//! A quote's contract value (R-165): the subscription it converts to, written and projected in a
//! transaction rolled back, valued as billing bills it.

use common_domain::actor::Actor;
use meteroid_store::domain::enums::SubscriptionFeeBillingPeriod;
use rstest::rstest;
use rust_decimal_macros::dec;

use super::phases::at;
use super::trial_offers::draft_quote;
use crate::data::ids::*;
use crate::harness::{TestEnv, test_env};

/// An escalator of 10 % every two months, twice, with no end date: 29.00 for January and
/// February, 31.90 for March and April, 35.09 from May, valued over 16 months (a year after the
/// last step). The revision published keeps the value; valuing leaves nothing behind.
/// Rules: R-165, R-155
#[rstest]
#[tokio::test]
async fn a_quote_is_valued_as_billing_bills_it(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::quote_signing::QuoteSigningInterface;
    use meteroid_store::repositories::quotes::QuotesInterface;
    let env = test_env.await;
    env.set_now(at(1, 1, 12));
    let quote_id = draft_quote(&env, Some(0), SubscriptionFeeBillingPeriod::Monthly).await;
    env.services()
        .set_quote_phases(
            TENANT_ID,
            quote_id,
            meteroid_store::domain::trial_offers::escalator(
                &[(COMP_STARTER_PLATFORM_FEE_ID, dec!(29.00))],
                dec!(10),
                2,
                2,
            ),
        )
        .await
        .expect("the quote's phases");

    let quote = env
        .store()
        .get_detailed_quote_by_id(TENANT_ID, quote_id)
        .await
        .expect("the quote");
    let value = env
        .services()
        .quote_totals(TENANT_ID, &quote)
        .await
        .expect("its totals")
        .contract
        .expect("its value");
    assert_eq!(value.term_months, None);
    assert_eq!(value.months, 16);
    assert_eq!(value.total, dec!(542.88));
    assert_eq!(value.committed_total, dec!(542.88));
    assert_eq!(value.annual, dec!(407.16));
    assert_eq!(value.years, vec![dec!(402.52), dec!(140.36)]);
    assert_eq!(
        value.phases,
        vec![Some(dec!(58.00)), Some(dec!(63.80)), None]
    );

    let stored = env
        .services()
        .quote_totals(TENANT_ID, &quote)
        .await
        .expect("its totals")
        .first_invoice_subtotal;
    assert_eq!(stored, Some(dec!(29.00)));

    env.store()
        .publish_quote(Actor::System, quote_id, TENANT_ID)
        .await
        .expect("published");
    env.store()
        .record_quote_revision(TENANT_ID, quote_id, None)
        .await
        .expect("its revision");
    let revision = env
        .store()
        .find_quote_revision(TENANT_ID, quote_id, quote.quote.revision)
        .await
        .expect("read")
        .expect("recorded");
    assert_eq!(revision.contract_value, Some(value));

    let quote = env
        .store()
        .get_detailed_quote_by_id(TENANT_ID, quote_id)
        .await
        .expect("the quote");
    assert_eq!(quote.quote.converted_to_subscription_id, None);
    env.store()
        .accept_quote(Actor::System, quote_id, TENANT_ID)
        .await
        .expect("accepted");
    env.services()
        .convert_quote_to_subscription(Actor::System, TENANT_ID, quote_id)
        .await
        .expect("a quote valued converts once");
}
