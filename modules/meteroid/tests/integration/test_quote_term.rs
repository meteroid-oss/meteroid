//! A quote's contract term (ADR-0022 §2): signed with the quote, written as its subscription's term.

use chrono::{Datelike, Months, NaiveDate};
use common_domain::actor::Actor;
use common_domain::ids::{QuoteId, SubscriptionId};
use meteroid_grpc::meteroid::api;
use meteroid_store::domain::subscription_phases::QuoteTerm;
use meteroid_store::repositories::QuotesInterface;
use rust_decimal_macros::dec;

use crate::data::ids;
use crate::harness::{TestEnv, test_env_with_seed};
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::SeedLevel;

async fn setup() -> (TestEnv, AllClients) {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(env.setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        env.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    (env, clients)
}

fn next_first_of_month() -> NaiveDate {
    let today = common_clock::now().date_naive();
    NaiveDate::from_ymd_opt(today.year(), today.month(), 1)
        .unwrap()
        .checked_add_months(Months::new(1))
        .unwrap()
}

async fn quote_starting(clients: &AllClients, start: NaiveDate, end: Option<NaiveDate>) -> QuoteId {
    let mut payload = crate::test_quote::test_quote_payload(5, None);
    payload.start_date = Some(start.to_string());
    payload.billing_day_anchor = Some(start.day());
    payload.end_date = end.map(|end| end.to_string());
    let quote = clients
        .quotes
        .clone()
        .create_quote(api::quotes::v1::CreateQuoteRequest {
            quote: Some(payload),
        })
        .await
        .expect("create quote")
        .into_inner()
        .quote
        .expect("quote");
    QuoteId::from_proto(&quote.quote.unwrap().id).unwrap()
}

fn three_years() -> QuoteTerm {
    QuoteTerm {
        term_months: 36,
        auto_renew: true,
        renewal_months: 12,
        notice_days: 60,
        uplift_percent: Some(dec!(5)),
    }
}

/// A quote for 3 years renewing yearly, 60 days' notice and +5%, converts into a subscription
/// with that term: its end 36 months after the billing start.
#[tokio::test]
async fn a_signed_term_is_the_subscription_term() {
    let (env, clients) = setup().await;
    let setup = &env.setup;
    let start = next_first_of_month();
    let quote_id = quote_starting(&clients, start, None).await;

    setup
        .services
        .set_quote_term(ids::TENANT_ID, quote_id, Some(three_years()))
        .await
        .expect("set term");
    let detailed = clients
        .quotes
        .clone()
        .get_quote(api::quotes::v1::GetQuoteRequest {
            id: quote_id.as_proto(),
        })
        .await
        .expect("get quote")
        .into_inner()
        .quote
        .expect("quote");
    let signed = detailed
        .contract_term
        .expect("the document states the term");
    assert_eq!((signed.term_months, signed.notice_days), (36, 60));

    clients
        .quotes
        .clone()
        .publish_quote(api::quotes::v1::PublishQuoteRequest {
            id: quote_id.as_proto(),
        })
        .await
        .expect("publish");
    setup
        .store
        .accept_quote(Actor::System, quote_id, ids::TENANT_ID)
        .await
        .expect("accept");
    let converted = clients
        .quotes
        .clone()
        .convert_quote_to_subscription(api::quotes::v1::ConvertQuoteToSubscriptionRequest {
            quote_id: quote_id.as_proto(),
        })
        .await
        .expect("convert")
        .into_inner();
    let subscription_id =
        SubscriptionId::from_proto(&converted.subscription.expect("subscription").id).unwrap();

    let term = setup
        .services
        .get_subscription_term(ids::TENANT_ID, subscription_id)
        .await
        .expect("read term")
        .expect("a term");
    assert_eq!(
        term.term_end,
        start.checked_add_months(Months::new(36)).unwrap()
    );
    assert!(term.auto_renew);
    assert_eq!(term.renewal_months, 12);
    assert_eq!(term.notice_days, 60);
    assert_eq!(term.uplift_percent, Some(dec!(5)));
    assert!(
        term.renewal_phase_id.is_none(),
        "written once the subscription starts"
    );

    env.run_until(start.succ_opt().unwrap()).await;
    let started = setup
        .services
        .get_subscription_term(ids::TENANT_ID, subscription_id)
        .await
        .expect("read term")
        .expect("a term");
    assert!(
        started.renewal_phase_id.is_some(),
        "the uplift renews from the term's end"
    );
    assert_eq!(started.term_end, term.term_end);
}

/// A term and an end date are one or the other; a term lasts 1 to 120 months.
#[tokio::test]
async fn a_term_is_refused_when_it_cannot_convert_as_signed() {
    let (env, clients) = setup().await;
    let setup = &env.setup;
    let start = next_first_of_month();
    let ended = quote_starting(
        &clients,
        start,
        Some(start.checked_add_months(Months::new(12)).unwrap()),
    )
    .await;
    let refused = setup
        .services
        .set_quote_term(ids::TENANT_ID, ended, Some(three_years()))
        .await
        .expect_err("a term with an end date");
    assert!(
        format!("{refused:?}").contains("remove its end date"),
        "{refused:?}"
    );

    let open = quote_starting(&clients, start, None).await;
    let too_long = QuoteTerm {
        term_months: 121,
        ..three_years()
    };
    assert!(
        setup
            .services
            .set_quote_term(ids::TENANT_ID, open, Some(too_long))
            .await
            .is_err()
    );
    let stored = setup
        .store
        .get_detailed_quote_by_id(ids::TENANT_ID, open)
        .await
        .unwrap();
    assert!(stored.term.is_none());
}
