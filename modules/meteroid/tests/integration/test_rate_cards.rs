//! Rate cards over gRPC: a provider's price feed synced in bulk, a rate per currency, and the
//! rate billing resolves.
use crate::data::ids::METRIC_BANDWIDTH;
use crate::meteroid_it::container::SeedLevel;
use crate::{helpers, meteroid_it};
use meteroid_grpc::meteroid::api::ratecards::v1 as grpc;
use tonic::Code;

fn rate(model: &str, kind: &str, currency: &str, rate: &str) -> grpc::RateCardRateInput {
    grpc::RateCardRateInput {
        key: vec![model.into(), kind.into()],
        currency: Some(currency.into()),
        rate: rate.into(),
        label: None,
    }
}

/// Rules: R-131
#[tokio::test]
async fn a_rate_card_syncs_in_bulk_and_rates_by_the_most_specific_key() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    let mut cards = clients.rate_cards.clone();

    let card = cards
        .create_rate_card(grpc::CreateRateCardRequest {
            code: "llm".into(),
            name: "LLM tokens".into(),
            metric_id: METRIC_BANDWIDTH.as_proto(),
            dimensions: vec!["model".into(), "token_type".into()],
            credit_unit_id: None,
            on_unmatched: grpc::RateCardUnmatched::Hold.into(),
            unit_size: Some("1000000".into()),
        })
        .await
        .expect("create")
        .into_inner()
        .card
        .expect("card");
    assert_eq!(card.unit_size, "1000000");
    assert_eq!(card.credit_unit_id, None, "a money card");

    let feed = vec![
        rate("anthropic/*", "input", "EUR", "3"),
        rate("anthropic/*", "input", "USD", "3.5"),
        rate("anthropic/claude-x", "input", "EUR", "2.5"),
        rate("*", "*", "EUR", "10"),
    ];
    let set = |effective_from: &str,
               rates: Vec<grpc::RateCardRateInput>,
               remove: Vec<grpc::RateCardKeyRemoval>| {
        let mut cards = cards.clone();
        let request = grpc::SetRateCardRatesRequest {
            rate_card_id: card.id.clone(),
            effective_from: Some(effective_from.into()),
            rates,
            remove,
        };
        async move { cards.set_rate_card_rates(request).await }
    };
    let first = set("2026-01-01", feed.clone(), vec![])
        .await
        .expect("first sync")
        .into_inner();
    assert_eq!((first.inserted, first.unchanged), (4, 0));
    let again = set("2026-01-01", feed.clone(), vec![])
        .await
        .expect("repeated sync")
        .into_inner();
    assert_eq!(
        (again.inserted, again.unchanged),
        (0, 4),
        "a repeated sync changes nothing"
    );

    let refused = set(
        "2026-01-01",
        vec![rate("anthropic/*", "input", "EUR", "4")],
        vec![],
    )
    .await;
    assert_eq!(
        refused.expect_err("a rate in force never changes").code(),
        Code::InvalidArgument
    );
    let no_currency = set(
        "2099-01-01",
        vec![grpc::RateCardRateInput {
            currency: None,
            ..rate("x", "input", "EUR", "1")
        }],
        vec![],
    )
    .await;
    assert_eq!(
        no_currency
            .expect_err("a money card's rate has a currency")
            .code(),
        Code::InvalidArgument
    );

    // One day for the batch: claude-x leaves EUR and its output is rated, from 2099.
    let batch = set(
        "2099-01-01",
        vec![rate("anthropic/claude-x", "output", "EUR", "12")],
        vec![grpc::RateCardKeyRemoval {
            key: vec!["anthropic/claude-x".into(), "input".into()],
            currency: Some("EUR".into()),
        }],
    )
    .await
    .expect("batch")
    .into_inner();
    assert_eq!((batch.inserted, batch.removed), (1, 1));

    let rate_of = |model: &str, kind: &str, currency: &str, on: &str| {
        let mut cards = cards.clone();
        let request = grpc::LookupRateRequest {
            rate_card_id: card.id.clone(),
            dimensions: [
                ("model".to_owned(), model.to_owned()),
                ("token_type".to_owned(), kind.to_owned()),
            ]
            .into_iter()
            .collect(),
            on: Some(on.into()),
            currency: Some(currency.into()),
        };
        async move {
            cards
                .lookup_rate(request)
                .await
                .expect("lookup")
                .into_inner()
                .entry
                .and_then(|entry| entry.rate)
        }
    };
    let june = "2026-06-01";
    assert_eq!(
        rate_of("anthropic/claude-x", "input", "EUR", june)
            .await
            .as_deref(),
        Some("2.5")
    );
    assert_eq!(
        rate_of("anthropic/claude-y", "input", "EUR", june)
            .await
            .as_deref(),
        Some("3")
    );
    assert_eq!(
        rate_of("anthropic/claude-x", "input", "USD", june)
            .await
            .as_deref(),
        Some("3.5"),
        "USD rates on their own"
    );
    assert_eq!(
        rate_of("openai/gpt", "output", "EUR", june)
            .await
            .as_deref(),
        Some("10")
    );
    assert_eq!(
        rate_of("openai/gpt", "output", "USD", june).await,
        None,
        "no rate in USD: unmatched"
    );
    let later = "2099-02-01";
    assert_eq!(
        rate_of("anthropic/claude-x", "input", "EUR", later)
            .await
            .as_deref(),
        Some("3"),
        "removed, claude-x falls to anthropic/*"
    );
    assert_eq!(
        rate_of("anthropic/claude-x", "output", "EUR", later)
            .await
            .as_deref(),
        Some("12")
    );

    let got = cards
        .get_rate_card(grpc::GetRateCardRequest {
            id: card.id.clone(),
            at: Some(june.into()),
        })
        .await
        .expect("get")
        .into_inner();
    assert_eq!(got.rates.len(), 4, "in force in June, both currencies");
    assert_eq!(
        got.entries.len(),
        6,
        "the 2099 rate and removal are entries too"
    );
    assert!(
        got.rates
            .iter()
            .any(|r| r.any_other && r.currency.as_deref() == Some("EUR")),
        "`*, *` is the Any other rate"
    );
}

/// Each write that changes anything is logged with who made it, latest first: a rate with the
/// one it follows, a removal, and a scheduled rate replaced before its day kept as it was.
#[tokio::test]
async fn a_rate_cards_changes_are_logged_with_who_made_them() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    let mut cards = clients.rate_cards.clone();

    let card = cards
        .create_rate_card(grpc::CreateRateCardRequest {
            code: "logged".into(),
            name: "Logged".into(),
            metric_id: METRIC_BANDWIDTH.as_proto(),
            dimensions: vec!["model".into(), "token_type".into()],
            credit_unit_id: None,
            on_unmatched: grpc::RateCardUnmatched::Hold.into(),
            unit_size: None,
        })
        .await
        .expect("create")
        .into_inner()
        .card
        .expect("card");
    let set = |effective_from: &str,
               rates: Vec<grpc::RateCardRateInput>,
               remove: Vec<grpc::RateCardKeyRemoval>| {
        let mut cards = cards.clone();
        let request = grpc::SetRateCardRatesRequest {
            rate_card_id: card.id.clone(),
            effective_from: Some(effective_from.into()),
            rates,
            remove,
        };
        async move { cards.set_rate_card_rates(request).await.expect("rates") }
    };
    set("2026-01-01", vec![rate("x", "input", "EUR", "3")], vec![]).await;
    set("2026-01-01", vec![rate("x", "input", "EUR", "3")], vec![]).await;
    set("2099-01-01", vec![rate("x", "input", "EUR", "4")], vec![]).await;
    set("2099-01-01", vec![rate("x", "input", "EUR", "5")], vec![]).await;
    set(
        "2098-01-01",
        vec![],
        vec![grpc::RateCardKeyRemoval {
            key: vec!["x".into(), "input".into()],
            currency: Some("EUR".into()),
        }],
    )
    .await;

    let changes = cards
        .list_rate_card_changes(grpc::ListRateCardChangesRequest {
            rate_card_id: card.id.clone(),
            cursor: None,
            limit: None,
        })
        .await
        .expect("changes")
        .into_inner()
        .changes;
    type Shown = (
        Option<String>,
        Vec<(Option<String>, Option<String>)>,
        Vec<String>,
    );
    let shown: Vec<Shown> = changes
        .iter()
        .map(|change| {
            (
                change.effective_from.clone(),
                change
                    .rates
                    .iter()
                    .map(|rate| (rate.from_rate.clone(), rate.to_rate.clone()))
                    .collect(),
                change
                    .withdrawn
                    .iter()
                    .map(|entry| {
                        format!(
                            "{} {}",
                            entry.effective_from,
                            entry.rate.clone().unwrap_or_default()
                        )
                    })
                    .collect(),
            )
        })
        .collect();
    let some = |s: &str| Some(s.to_string());
    assert_eq!(
        shown,
        vec![
            (
                some("2098-01-01"),
                vec![(some("3"), None)],
                vec!["2099-01-01 5".to_string()]
            ),
            (
                some("2099-01-01"),
                vec![(some("3"), some("5"))],
                vec!["2099-01-01 4".to_string()]
            ),
            (some("2099-01-01"), vec![(some("3"), some("4"))], vec![]),
            (some("2026-01-01"), vec![(None, some("3"))], vec![]),
            (None, vec![], vec![]),
        ],
        "the repeated sync is not logged"
    );
    assert_eq!(
        changes.last().map(|change| change.kind),
        Some(grpc::RateCardChangeKind::Created.into())
    );
    assert!(changes.iter().all(|change| {
        change.actor_type == meteroid_grpc::meteroid::api::activity::v1::ActorType::User as i32
            && change.actor_name.is_some()
    }));
}

/// A metric declaring its dimension values (a segmentation matrix) is rated by a card like any
/// other: the values only suggest keys.
/// Rules: R-136
#[tokio::test]
async fn a_rate_card_rates_a_metric_declaring_its_dimension_values() {
    use common_domain::actor::Actor;
    use meteroid_store::domain::BillableMetricNew;
    use meteroid_store::domain::billable_metrics::{Dimension, SegmentationMatrix};
    use meteroid_store::domain::enums::BillingMetricAggregateEnum;
    use meteroid_store::repositories::billable_metrics::BillableMetricInterface;

    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    let metric = setup
        .store
        .insert_billable_metric(
            Actor::System,
            BillableMetricNew {
                attributed_to: None,
                name: "Segmented tokens".to_owned(),
                description: None,
                code: "segmented_tokens".to_owned(),
                catalog_code: None,
                aggregation_type: BillingMetricAggregateEnum::Sum,
                aggregation_key: Some("tokens".to_owned()),
                unit_conversion_factor: None,
                unit_conversion_rounding: None,
                segmentation_matrix: Some(SegmentationMatrix::Single(Dimension {
                    key: "model".to_owned(),
                    values: vec!["gpt-5".to_owned(), "claude".to_owned()],
                })),
                filters: None,
                usage_group_key: None,
                usage_breakdown_key: None,
                unit_label: None,
                tenant_id: crate::data::ids::TENANT_ID,
                product_family_id: crate::data::ids::PRODUCT_FAMILY_ID,
                product_id: None,
            },
        )
        .await
        .expect("metric");

    let card = clients
        .rate_cards
        .clone()
        .create_rate_card(grpc::CreateRateCardRequest {
            code: "segmented".into(),
            name: "Segmented".into(),
            metric_id: metric.id.as_proto(),
            dimensions: vec!["model".into()],
            credit_unit_id: None,
            on_unmatched: grpc::RateCardUnmatched::Hold.into(),
            unit_size: None,
        })
        .await
        .expect("a card rates a metric with a segmentation matrix")
        .into_inner()
        .card
        .expect("card");
    assert_eq!(card.dimensions, vec!["model".to_string()]);
}

/// Cost cards are an Enterprise module's: core's rate card APIs list, read, rate and log price
/// cards only, and a cost card reads as not found.
#[tokio::test]
async fn a_cost_card_is_absent_from_cores_rate_card_apis() {
    use common_domain::actor::Actor;
    use meteroid_store::domain::rate_cards::{
        RateCardDenomination, RateCardNew, RateCardPurpose, RateCardUnmatched,
    };
    use meteroid_store::repositories::RateCardsInterface;

    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = meteroid_it::clients::AllClients::from_channel(
        setup.channel.clone(),
        auth.token.clone().as_str(),
        "TESTORG",
        "testslug",
    );
    let mut cards = clients.rate_cards.clone();
    let cost = setup
        .store
        .create_rate_card(
            Actor::System,
            RateCardNew {
                tenant_id: crate::data::ids::TENANT_ID,
                code: "bandwidth-costs".into(),
                name: "Bandwidth costs".into(),
                metric_id: METRIC_BANDWIDTH,
                dimensions: vec!["region".into()],
                denomination: RateCardDenomination::Money,
                unit_size: rust_decimal::Decimal::ONE,
                on_unmatched: RateCardUnmatched::Ignore,
                purpose: RateCardPurpose::Cost,
            },
        )
        .await
        .expect("a cost card");

    let listed = cards
        .list_rate_cards(grpc::ListRateCardsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .cards;
    assert!(listed.iter().all(|card| card.id != cost.id.as_proto()));
    let get = cards
        .get_rate_card(grpc::GetRateCardRequest {
            id: cost.id.as_proto(),
            at: None,
        })
        .await
        .unwrap_err();
    assert_eq!(get.code(), Code::NotFound);
    let set = cards
        .set_rate_card_rates(grpc::SetRateCardRatesRequest {
            rate_card_id: cost.id.as_proto(),
            effective_from: None,
            rates: vec![grpc::RateCardRateInput {
                key: vec!["eu".into()],
                currency: Some("EUR".into()),
                rate: "1".into(),
                label: None,
            }],
            remove: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(set.code(), Code::NotFound);
    let lookup = cards
        .lookup_rate(grpc::LookupRateRequest {
            rate_card_id: cost.id.as_proto(),
            dimensions: Default::default(),
            on: None,
            currency: Some("EUR".into()),
        })
        .await
        .unwrap_err();
    assert_eq!(lookup.code(), Code::NotFound);
    let changes = cards
        .list_rate_card_changes(grpc::ListRateCardChangesRequest {
            rate_card_id: cost.id.as_proto(),
            cursor: None,
            limit: None,
        })
        .await
        .unwrap_err();
    assert_eq!(changes.code(), Code::NotFound);

    let api_key = clients
        .api_tokens
        .clone()
        .create_api_token(
            meteroid_grpc::meteroid::api::apitokens::v1::CreateApiTokenRequest {
                name: "rate cards".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .into_inner()
        .api_key;
    let http = http_client_tracing::client();
    let base = setup.config.rest_api_external_url.clone();
    let get = |path: String| {
        let request = http.get(format!("{base}{path}")).bearer_auth(&api_key);
        async move { request.send().await.unwrap() }
    };
    let list: serde_json::Value = get("/api/v2/rate-cards".into()).await.json().await.unwrap();
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|card| card["code"] != "bandwidth-costs"),
        "{list}"
    );
    for path in [
        "/api/v2/rate-cards/bandwidth-costs",
        "/api/v2/rate-cards/bandwidth-costs/changes",
        "/api/v1/rate-cards/bandwidth-costs",
    ] {
        assert_eq!(get(path.into()).await.status(), 404, "{path}");
    }
}
