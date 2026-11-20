//! Quotes v2 in core: drafts, line discounts, revisions, the signing flow and its evidence,
//! countersignatures, approvals. Gates are open here (`core_test_modules`); refusals are tested
//! in the cpq module.
use crate::data::ids::{COMP_NOTION_SEATS_ID, CUST_SPOTIFY_ID, PLAN_VERSION_NOTION_ID, TENANT_ID};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::clients::AllClients;
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use crate::meteroid_it::svc_auth::SEED_USERNAME;
use common_domain::ids::{BaseId, QuoteApprovalRuleId, QuoteId, UserId};
use diesel_models::quote_approvals::QuoteApprovalRuleRowNew;
use meteroid_grpc::meteroid::api;
use meteroid_grpc::meteroid::api::quotes::v1::quote_line_detail::Target;
use meteroid_grpc::meteroid::portal::quotes::v1 as portal;
use meteroid_grpc::meteroid::portal::quotes::v1::portal_quote_service_client::PortalQuoteServiceClient;
use meteroid_store::repositories::quote_approvals::QuoteApprovalsInterface;
use tonic::Code;

struct Env {
    setup: MeteroidSetup,
    clients: AllClients,
}

async fn env() -> Env {
    helpers::init::logging();
    let database = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await;
    let auth = meteroid_it::svc_auth::login(setup.channel.clone()).await;
    let clients = AllClients::from_channel(
        setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    Env { setup, clients }
}

const SIGNER: &str = "jane@spotify.test";

fn quote() -> api::quotes::v1::CreateQuote {
    api::quotes::v1::CreateQuote {
        plan_version_id: PLAN_VERSION_NOTION_ID.as_proto(),
        customer_id: CUST_SPOTIFY_ID.as_proto(),
        currency: "EUR".to_owned(),
        start_date: Some(chrono::Utc::now().date_naive().to_string()),
        billing_day_anchor: Some(1),
        activation_condition: api::subscriptions::v1::ActivationCondition::OnStart.into(),
        net_terms: Some(30),
        recipients: vec![api::quotes::v1::RecipientDetails {
            name: "Jane".to_owned(),
            email: SIGNER.to_owned(),
        }],
        components: Some(api::subscriptions::v1::CreateSubscriptionComponents {
            parameterized_components: vec![
                api::subscriptions::v1::create_subscription_components::ComponentParameterization {
                    component_id: COMP_NOTION_SEATS_ID.as_proto(),
                    billing_period: Some(api::shared::v1::BillingPeriod::Monthly.into()),
                    initial_slot_count: Some(5),
                    committed_capacity: None,
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn discounted(percent: &str) -> api::quotes::v1::CreateQuote {
    api::quotes::v1::CreateQuote {
        line_details: vec![api::quotes::v1::QuoteLineDetail {
            target: Some(Target::PriceComponentId(COMP_NOTION_SEATS_ID.as_proto())),
            description: Some("Seats for the whole team".to_owned()),
            sort_order: 1,
            discount_percent: Some(percent.to_owned()),
        }],
        ..quote()
    }
}

fn with_token<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().unwrap());
    request
}

impl Env {
    async fn create(&self, quote: api::quotes::v1::CreateQuote) -> api::quotes::v1::DetailedQuote {
        self.clients
            .quotes
            .clone()
            .create_quote(api::quotes::v1::CreateQuoteRequest { quote: Some(quote) })
            .await
            .unwrap()
            .into_inner()
            .quote
            .unwrap()
    }

    async fn get(&self, id: &str) -> api::quotes::v1::DetailedQuote {
        self.clients
            .quotes
            .clone()
            .get_quote(api::quotes::v1::GetQuoteRequest { id: id.to_owned() })
            .await
            .unwrap()
            .into_inner()
            .quote
            .unwrap()
    }

    async fn publish(&self, id: &str) -> Result<(), tonic::Status> {
        self.clients
            .quotes
            .clone()
            .publish_quote(api::quotes::v1::PublishQuoteRequest { id: id.to_owned() })
            .await
            .map(|_| ())
    }

    async fn portal_token(&self, id: &str, email: &str) -> String {
        self.clients
            .quotes
            .clone()
            .generate_quote_portal_token(api::quotes::v1::GenerateQuotePortalTokenRequest {
                quote_id: id.to_owned(),
                recipient_email: email.to_owned(),
            })
            .await
            .unwrap()
            .into_inner()
            .token
    }

    fn portal(&self) -> PortalQuoteServiceClient<tonic::transport::Channel> {
        PortalQuoteServiceClient::new(self.setup.channel.clone())
    }

    async fn sign(
        &self,
        token: &str,
        hash: Option<String>,
    ) -> Result<portal::SignQuoteResponse, tonic::Status> {
        self.portal()
            .sign_quote(with_token(
                portal::SignQuoteRequest {
                    recipient_email: SIGNER.to_owned(),
                    signed_by_name: "Jane".to_owned(),
                    signed_by_title: "CFO".to_owned(),
                    signature_data: "data:image/png;base64,AAAA".to_owned(),
                    signature_method: "drawn".to_owned(),
                    document_hash: hash,
                    consent_text: Some("I accept".to_owned()),
                },
                token,
            ))
            .await
            .map(tonic::Response::into_inner)
    }

    async fn evidence(&self, id: &str) -> api::quotes::v1::ListQuoteEvidenceResponse {
        self.clients
            .quotes
            .clone()
            .list_quote_evidence(api::quotes::v1::ListQuoteEvidenceRequest {
                quote_id: id.to_owned(),
            })
            .await
            .unwrap()
            .into_inner()
    }
}

#[tokio::test]
async fn a_draft_keeps_the_latest_save_and_refuses_a_stale_one() {
    let env = env().await;
    let mut quotes = env.clients.quotes.clone();
    let save = |id: Option<String>, expected: Option<i32>, payload: &str| {
        api::quotes::v1::SaveQuoteDraftRequest {
            id,
            quote_id: None,
            expected_version: expected,
            label: Some("Spotify renewal".to_owned()),
            payload: payload.to_owned(),
            payload_version: 1,
        }
    };

    let first = quotes
        .save_quote_draft(save(None, None, r#"{"step":1}"#))
        .await
        .unwrap()
        .into_inner();
    assert!(!first.conflict);
    let draft = first.draft.unwrap();
    assert_eq!(draft.version, 1);

    let second = quotes
        .save_quote_draft(save(Some(draft.id.clone()), Some(1), r#"{"step":2}"#))
        .await
        .unwrap()
        .into_inner();
    assert!(!second.conflict);
    assert_eq!(second.draft.as_ref().unwrap().version, 2);

    // Another tab still on version 1 gets the newer draft back, unchanged.
    let stale = quotes
        .save_quote_draft(save(Some(draft.id.clone()), Some(1), r#"{"step":"stale"}"#))
        .await
        .unwrap()
        .into_inner();
    assert!(stale.conflict);
    assert_eq!(stale.draft.as_ref().unwrap().payload, r#"{"step":2}"#);

    let listed = quotes
        .list_quote_drafts(api::quotes::v1::ListQuoteDraftsRequest {})
        .await
        .unwrap()
        .into_inner()
        .drafts;
    assert_eq!(listed.len(), 1);

    let refused = quotes
        .save_quote_draft(save(None, None, "not json"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);

    quotes
        .delete_quote_draft(api::quotes::v1::DeleteQuoteDraftRequest { id: draft.id })
        .await
        .unwrap();
    let listed = quotes
        .list_quote_drafts(api::quotes::v1::ListQuoteDraftsRequest {})
        .await
        .unwrap()
        .into_inner()
        .drafts;
    assert!(listed.is_empty());
}

#[tokio::test]
async fn a_line_discount_lowers_the_billed_fee_and_keeps_the_list_price() {
    let env = env().await;
    let created = env.create(discounted("20")).await;
    let seats = created
        .components
        .iter()
        .find(|c| c.price_component_id.as_deref() == Some(COMP_NOTION_SEATS_ID.as_proto().as_str()))
        .unwrap();

    let unit_rate = |fee: &api::subscriptions::v1::SubscriptionFee| match &fee.fee {
        Some(api::subscriptions::v1::subscription_fee::Fee::Slot(slot)) => {
            slot.unit_rate.parse::<rust_decimal::Decimal>().unwrap()
        }
        other => panic!("seats are a slot fee, got {other:?}"),
    };
    let list = unit_rate(seats.list_fee.as_ref().unwrap());
    let net = unit_rate(seats.fee.as_ref().unwrap());
    assert_eq!(net, (list * rust_decimal::Decimal::new(8, 1)).round_dp(2));
    assert_eq!(seats.discount_percent.as_deref(), Some("20"));
    assert_eq!(
        seats.description.as_deref(),
        Some("Seats for the whole team")
    );
    assert!(seats.is_override);

    let quote = created.quote.unwrap();
    assert_eq!(quote.max_discount_percent.as_deref(), Some("20"));
    assert!(quote.document_hash.is_some());

    let refused = env
        .clients
        .quotes
        .clone()
        .create_quote(api::quotes::v1::CreateQuoteRequest {
            quote: Some(discounted("120")),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn editing_a_published_quote_makes_a_new_revision() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    env.publish(&created.id).await.unwrap();
    let before = env.get(&created.id).await.quote.unwrap();

    env.clients
        .quotes
        .clone()
        .update_quote(api::quotes::v1::UpdateQuoteRequest {
            id: created.id.clone(),
            quote: Some(discounted("10")),
            expected_updated_at: None,
        })
        .await
        .unwrap();
    let after = env.get(&created.id).await.quote.unwrap();
    assert_eq!(after.revision, before.revision + 1);
    assert_ne!(after.document_hash, before.document_hash);

    let revisions = env
        .clients
        .quotes
        .clone()
        .list_quote_revisions(api::quotes::v1::ListQuoteRevisionsRequest {
            quote_id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .revisions;
    assert_eq!(
        revisions.iter().map(|r| r.revision).collect::<Vec<_>>(),
        vec![after.revision, before.revision]
    );
    assert!(revisions[1].superseded_at.is_some());
    assert!(revisions[0].superseded_at.is_none());
    assert_eq!(
        Some(revisions[0].document_hash.clone()),
        after.document_hash
    );
}

#[tokio::test]
async fn a_signature_binds_the_version_it_was_shown() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    env.publish(&created.id).await.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;

    let shown = env
        .portal()
        .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap();
    let hash = shown.quote.unwrap().document_hash;
    assert!(hash.is_some());

    let stale = env.sign(&token, Some("0".repeat(64))).await.unwrap_err();
    assert_eq!(stale.code(), Code::InvalidArgument);

    let signed = env.sign(&token, hash).await.unwrap();
    assert!(signed.accepted);
    assert!(!signed.awaiting_countersignature);

    let detailed = env.get(&created.id).await;
    assert_eq!(
        detailed.quote.unwrap().status(),
        api::quotes::v1::QuoteStatus::Accepted
    );
    let signature = &detailed.signatures[0];
    assert_eq!(signature.consent_text.as_deref(), Some("I accept"));
    assert!(signature.document_hash.is_some());

    let evidence = env.evidence(&created.id).await;
    assert!(evidence.chain_intact);
    let kinds: Vec<&str> = evidence.events.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, vec!["published", "viewed", "signed", "accepted"]);

    // The PDF and its signature pages come with the quote-documents module only.
    assert!(env.get(&created.id).await.pdf_url.is_none());
}

#[tokio::test]
async fn a_verified_signature_needs_a_confirmed_code() {
    let env = env().await;
    let (id, token) = env.verified_quote().await;
    let hash = env.get(&id).await.quote.unwrap().document_hash;

    // The codes are the cpq module's; core still refuses a verified signature without one.
    let without_code = env.sign(&token, hash).await.unwrap_err();
    assert_eq!(without_code.code(), Code::InvalidArgument);
    assert!(
        env.evidence(&id)
            .await
            .events
            .iter()
            .all(|e| e.kind != "signed")
    );
}

#[tokio::test]
async fn a_large_discount_needs_an_approval_before_publishing() {
    let env = env().await;
    let mut conn = env.setup.store.get_conn().await.unwrap();
    QuoteApprovalRuleRowNew {
        id: QuoteApprovalRuleId::new(),
        tenant_id: TENANT_ID,
        name: "Discounts over 15%".to_owned(),
        min_discount_percent: Some(rust_decimal::Decimal::new(15, 0)),
        max_net_terms_days: None,
        max_trial_days: None,
        always: false,
        approver_user_ids: vec![],
    }
    .insert(&mut conn)
    .await
    .unwrap();
    drop(conn);

    let small = env.create(discounted("10")).await.quote.unwrap();
    env.publish(&small.id).await.unwrap();
    let token = env.portal_token(&small.id, SIGNER).await;

    // Raising the discount of a published quote takes it back to draft until approved.
    env.clients
        .quotes
        .clone()
        .update_quote(api::quotes::v1::UpdateQuoteRequest {
            id: small.id.clone(),
            quote: Some(discounted("30")),
            expected_updated_at: None,
        })
        .await
        .unwrap();
    assert_eq!(
        env.get(&small.id).await.quote.unwrap().status(),
        api::quotes::v1::QuoteStatus::Draft
    );
    let held = env
        .portal()
        .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
        .await
        .unwrap_err();
    assert_eq!(
        held.code(),
        Code::FailedPrecondition,
        "unapproved terms stay hidden"
    );
    let revisions = env
        .clients
        .quotes
        .clone()
        .list_quote_revisions(api::quotes::v1::ListQuoteRevisionsRequest {
            quote_id: small.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .revisions;
    assert_eq!(
        revisions.iter().map(|r| r.revision).collect::<Vec<_>>(),
        vec![1],
        "a held revision is recorded once published"
    );

    let large = env.create(discounted("25")).await.quote.unwrap();
    let refused = env.publish(&large.id).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);

    let quote_id = QuoteId::from_proto(&large.id).unwrap();
    let approval = env
        .setup
        .store
        .request_quote_approval(TENANT_ID, quote_id, UserId::new(), None)
        .await
        .unwrap();
    assert_eq!(
        env.publish(&large.id).await.unwrap_err().code(),
        Code::FailedPrecondition
    );

    env.setup
        .store
        .decide_quote_approval(TENANT_ID, approval.id, UserId::new(), true, None)
        .await
        .unwrap();
    let status = env
        .clients
        .quotes
        .clone()
        .get_quote_approval(api::quotes::v1::GetQuoteApprovalRequest {
            quote_id: large.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(status.satisfied);
    env.publish(&large.id).await.unwrap();
}

#[tokio::test]
async fn a_quote_links_to_a_hubspot_deal_by_id_or_url() {
    let env = env().await;
    let linked = env
        .create(api::quotes::v1::CreateQuote {
            hubspot_deal_id: Some(
                "https://app.hubspot.com/contacts/1/record/0-3/987654".to_owned(),
            ),
            ..quote()
        })
        .await
        .quote
        .unwrap();
    assert_eq!(linked.hubspot_deal_id.as_deref(), Some("987654"));

    let refused = env
        .clients
        .quotes
        .clone()
        .create_quote(api::quotes::v1::CreateQuoteRequest {
            quote: Some(api::quotes::v1::CreateQuote {
                hubspot_deal_id: Some("not a deal".to_owned()),
                ..quote()
            }),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn a_stale_save_is_refused_unless_forced() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    let update = |expected: Option<String>| api::quotes::v1::UpdateQuoteRequest {
        id: created.id.clone(),
        quote: Some(quote()),
        expected_updated_at: expected,
    };
    let mut quotes = env.clients.quotes.clone();

    let loaded = created.updated_at.clone().unwrap_or_default();
    quotes
        .update_quote(update(Some(loaded.clone())))
        .await
        .unwrap();

    // A second editor still holding the first version.
    let stale = quotes.update_quote(update(Some(loaded))).await.unwrap_err();
    assert_eq!(stale.code(), Code::Aborted);

    // Overwriting stays possible.
    quotes.update_quote(update(None)).await.unwrap();

    let current = env.get(&created.id).await.quote.unwrap().updated_at;
    quotes.update_quote(update(current)).await.unwrap();
}

#[tokio::test]
async fn a_change_request_marks_the_quote_until_the_next_revision() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;
    let write = |body: &str, request_changes: bool| {
        let mut portal = env.portal();
        let request = with_token(
            portal::AddQuoteCommentRequest {
                body: body.to_owned(),
                request_changes,
                anchor: None,
            },
            &token,
        );
        async move { portal.add_quote_comment(request).await }
    };

    let draft = write("Hello", false).await.unwrap_err();
    assert_eq!(draft.code(), Code::InvalidArgument, "not published yet");

    env.publish(&created.id).await.unwrap();
    let loaded = env.get(&created.id).await.quote.unwrap().updated_at;
    let shown = env
        .portal()
        .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap();
    assert!(shown.discussion_enabled);
    assert!(shown.comments.is_empty());

    let empty = write("   ", false).await.unwrap_err();
    assert_eq!(empty.code(), Code::InvalidArgument);

    let request = write("Could the seats start at 20?", true)
        .await
        .unwrap()
        .into_inner()
        .comment
        .unwrap();
    assert!(request.from_customer && request.shared && request.is_change_request);
    assert_eq!(request.author_name, "Jane");
    assert_eq!(request.author_email.as_deref(), Some(SIGNER));

    let marked = env.get(&created.id).await.quote.unwrap();
    assert!(marked.changes_requested_at.is_some());
    let evidence = env.evidence(&created.id).await;
    assert!(evidence.chain_intact);
    let change = evidence
        .events
        .iter()
        .find(|e| e.kind == "changes_requested")
        .expect("the change request is in the evidence");
    assert_eq!(change.actor_email.as_deref(), Some(SIGNER));

    env.clients
        .quotes
        .clone()
        .update_quote(api::quotes::v1::UpdateQuoteRequest {
            id: created.id.clone(),
            quote: Some(discounted("10")),
            expected_updated_at: loaded,
        })
        .await
        .expect("a change request does not make the editor's copy stale");

    let revised = env.get(&created.id).await.quote.unwrap();
    assert_eq!(revised.revision, marked.revision + 1);
    assert!(revised.changes_requested_at.is_none());
    let comments = env
        .portal()
        .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap()
        .comments;
    assert_eq!(comments.len(), 1);
    assert!(comments[0].resolved_at.is_some());
}

#[tokio::test]
async fn the_portal_leaves_out_the_sellers_own_data() {
    let env = env().await;
    let created = env
        .create(api::quotes::v1::CreateQuote {
            countersigners: vec![api::quotes::v1::QuoteCountersigner {
                name: "Sam Seller".to_owned(),
                email: "sam@seller.test".to_owned(),
                title: None,
                automatic: true,
            }],
            ..discounted("10")
        })
        .await
        .quote
        .unwrap();
    env.publish(&created.id).await.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;
    let shown = || async {
        env.portal()
            .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
            .await
            .unwrap()
            .into_inner()
            .quote
            .unwrap()
    };

    let before = shown().await;
    let quote = before.quote.unwrap();
    assert!(quote.countersigners.is_empty());
    assert!(quote.max_discount_percent.is_none() && quote.owner_id.is_none());
    assert_eq!(before.countersigners[0].name, "Sam Seller");

    let signed = env.sign(&token, quote.document_hash).await.unwrap();
    assert!(
        signed.accepted,
        "the automatic countersignature completes it"
    );
    let after = shown().await;
    let countersignature = after
        .signatures
        .iter()
        .find(|s| s.signed_by_name == "Sam Seller")
        .expect("the countersignature is listed");
    assert!(countersignature.signed_by_email.is_empty());
    assert!(after.signatures.iter().any(|s| s.signed_by_email == SIGNER));
}

#[tokio::test]
async fn declining_joins_the_evidence() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    env.publish(&created.id).await.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;
    env.portal()
        .decline_quote(with_token(
            portal::DeclineQuoteRequest {
                reason: Some("Too expensive".to_owned()),
            },
            &token,
        ))
        .await
        .unwrap();
    let evidence = env.evidence(&created.id).await;
    assert!(evidence.chain_intact);
    let declined = evidence.events.last().unwrap();
    assert_eq!(declined.kind, "declined");
    assert_eq!(declined.actor_email.as_deref(), Some(SIGNER));
}

#[tokio::test]
async fn reopening_within_half_an_hour_is_one_view() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    env.publish(&created.id).await.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;
    let open = || async {
        env.portal()
            .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
            .await
            .unwrap();
    };
    let engagement = || async {
        let engagement = env.get(&created.id).await.engagement.unwrap();
        assert_eq!(engagement.recipients.len(), 1, "one entry per person");
        engagement.recipients.into_iter().next().unwrap()
    };
    let at = |instant: &str| chrono::DateTime::parse_from_rfc3339(instant).unwrap();

    assert!(
        env.get(&created.id)
            .await
            .engagement
            .unwrap()
            .recipients
            .is_empty()
    );

    let revision = env.get(&created.id).await.quote.unwrap().revision;
    open().await;
    let first = engagement().await;
    assert_eq!(first.email, SIGNER);
    assert_eq!(first.view_count, 1);
    assert_eq!(first.first_viewed_at, first.last_viewed_at);
    assert_eq!(first.last_viewed_revision, revision as u32);

    common_clock::test::advance(chrono::Duration::minutes(1));
    open().await;
    let reopened = engagement().await;
    assert_eq!(reopened.view_count, 1, "a reload is the same view");
    assert!(at(&reopened.last_viewed_at) > at(&first.last_viewed_at));
    assert_eq!(reopened.first_viewed_at, first.first_viewed_at);

    common_clock::test::advance(chrono::Duration::minutes(31));
    open().await;
    let returned = engagement().await;
    assert_eq!(returned.view_count, 2);

    let typed = env
        .portal_token(&created.id, &format!(" {} ", SIGNER.to_uppercase()))
        .await;
    assert_eq!(typed, token, "the link is minted for the stored email");
    open().await;
    let same_person = engagement().await;
    assert_eq!(same_person.email, SIGNER, "the first spelling is kept");
    assert_eq!(same_person.view_count, 2);

    let evidence = env.evidence(&created.id).await;
    assert!(evidence.chain_intact);
    let views = evidence
        .events
        .iter()
        .filter(|e| e.kind == "viewed")
        .count();
    assert_eq!(views, 1, "the evidence keeps one view per revision");
}

const MAC_CHROME: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0.0.0 Safari/537.36";
const IPHONE_SAFARI: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";

/// The client IP is read from either header, whatever the proxy hops configured.
async fn open_portal_from(env: &Env, token: &str, ip: &str, user_agent: &str) {
    let mut request = with_token(portal::GetQuotePortalRequest {}, token);
    let metadata = request.metadata_mut();
    metadata.insert("x-forwarded-for", ip.parse().unwrap());
    metadata.insert("x-real-ip", ip.parse().unwrap());
    metadata.insert("user-agent", user_agent.parse().unwrap());
    env.portal().get_quote_portal(request).await.unwrap();
}

#[tokio::test]
async fn reopening_within_half_an_hour_extends_the_view_session() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    env.publish(&created.id).await.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;
    let sessions = || async { env.get(&created.id).await.engagement.unwrap().sessions };
    let at = |instant: &str| chrono::DateTime::parse_from_rfc3339(instant).unwrap();

    open_portal_from(&env, &token, "203.0.113.7", MAC_CHROME).await;
    let first = sessions().await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].viewed_at, first[0].last_seen_at);

    common_clock::test::advance(chrono::Duration::minutes(1));
    open_portal_from(&env, &token, "203.0.113.7", MAC_CHROME).await;
    let reopened = sessions().await;
    assert_eq!(reopened.len(), 1, "a reload is the same session");
    assert_eq!(reopened[0].viewed_at, first[0].viewed_at);
    assert!(at(&reopened[0].last_seen_at) > at(&first[0].last_seen_at));

    common_clock::test::advance(chrono::Duration::minutes(31));
    open_portal_from(&env, &token, "203.0.113.7", MAC_CHROME).await;
    let returned = sessions().await;
    assert_eq!(returned.len(), 2);
    assert!(
        at(&returned[0].viewed_at) > at(&returned[1].last_seen_at),
        "newest first"
    );
    assert_eq!(returned[1].viewed_at, first[0].viewed_at);

    let engagement = env.get(&created.id).await.engagement.unwrap();
    assert_eq!(
        engagement.recipients[0].view_count as usize,
        returned.len(),
        "one view per session"
    );
}

#[tokio::test]
async fn a_view_session_keeps_the_ip_and_device() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    env.publish(&created.id).await.unwrap();
    let revision = env.get(&created.id).await.quote.unwrap().revision as u32;
    let token = env.portal_token(&created.id, SIGNER).await;

    open_portal_from(&env, &token, "203.0.113.7", MAC_CHROME).await;
    common_clock::test::advance(chrono::Duration::minutes(31));
    open_portal_from(&env, &token, "198.51.100.4", IPHONE_SAFARI).await;

    let sessions = env.get(&created.id).await.engagement.unwrap().sessions;
    assert_eq!(sessions.len(), 2);
    assert!(
        sessions
            .iter()
            .all(|s| s.email == SIGNER && s.revision == revision)
    );
    assert_eq!(sessions[0].ip_address.as_deref(), Some("198.51.100.4"));
    assert_eq!(sessions[0].device.as_deref(), Some("Safari on iOS"));
    assert_eq!(sessions[1].ip_address.as_deref(), Some("203.0.113.7"));
    assert_eq!(sessions[1].device.as_deref(), Some("Chrome on macOS"));
}

fn restricted(
    questions: bool,
    change_requests: bool,
    decline: bool,
) -> api::quotes::v1::CreateQuote {
    api::quotes::v1::CreateQuote {
        customer_actions: Some(api::quotes::v1::QuoteCustomerActions {
            questions,
            change_requests,
            decline,
        }),
        ..quote()
    }
}

fn actions_of(quote: &api::quotes::v1::Quote) -> (bool, bool, bool) {
    let actions = quote.customer_actions.as_ref().unwrap();
    (actions.questions, actions.change_requests, actions.decline)
}

#[tokio::test]
async fn customer_actions_default_to_all_and_follow_each_save() {
    let env = env().await;
    let open = env.create(quote()).await.quote.unwrap();
    assert_eq!(actions_of(&open), (true, true, true));

    let created = env
        .create(restricted(false, true, false))
        .await
        .quote
        .unwrap();
    assert_eq!(actions_of(&created), (false, true, false));
    assert_eq!(
        actions_of(&env.get(&created.id).await.quote.unwrap()),
        (false, true, false)
    );

    env.publish(&created.id).await.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;
    let shown = env
        .portal()
        .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap();
    assert!(!shown.discussion_enabled);
    assert!(shown.change_requests_enabled);
    assert!(!shown.decline_enabled);

    // The editor sends the whole quote: no actions is every action.
    env.clients
        .quotes
        .clone()
        .update_quote(api::quotes::v1::UpdateQuoteRequest {
            id: created.id.clone(),
            quote: Some(quote()),
            expected_updated_at: None,
        })
        .await
        .unwrap();
    assert_eq!(
        actions_of(&env.get(&created.id).await.quote.unwrap()),
        (true, true, true)
    );
}

#[tokio::test]
async fn the_portal_refuses_the_actions_the_seller_turned_off() {
    let env = env().await;
    let created = env
        .create(restricted(false, false, false))
        .await
        .quote
        .unwrap();
    env.publish(&created.id).await.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;
    let write = |request_changes: bool| {
        let mut portal = env.portal();
        let request = with_token(
            portal::AddQuoteCommentRequest {
                body: "Could the seats start at 20?".to_owned(),
                request_changes,
                anchor: None,
            },
            &token,
        );
        async move { portal.add_quote_comment(request).await }
    };

    let question = write(false).await.unwrap_err();
    assert_eq!(question.code(), Code::FailedPrecondition);
    let change_request = write(true).await.unwrap_err();
    assert_eq!(change_request.code(), Code::FailedPrecondition);

    let declined = env
        .portal()
        .decline_quote(with_token(
            portal::DeclineQuoteRequest {
                reason: Some("Too expensive".to_owned()),
            },
            &token,
        ))
        .await
        .unwrap_err();
    assert_eq!(declined.code(), Code::FailedPrecondition);
    assert_eq!(
        env.get(&created.id).await.quote.unwrap().status,
        api::quotes::v1::QuoteStatus::Pending as i32
    );

    // The customer refused by other means: the seller records it.
    let recorded = env
        .clients
        .quotes
        .clone()
        .decline_quote(api::quotes::v1::DeclineQuoteRequest {
            id: created.id.clone(),
            reason: Some("Refused on a call".to_owned()),
        })
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap()
        .quote
        .unwrap();
    assert_eq!(
        recorded.status,
        api::quotes::v1::QuoteStatus::Declined as i32
    );
}

#[tokio::test]
async fn a_quote_totals_its_discounted_lines() {
    use chrono::Datelike;

    let env = env().await;
    let start = chrono::Utc::now().date_naive();
    let payload = api::quotes::v1::CreateQuote {
        // Billed from its own day, the first period is a full one.
        billing_day_anchor: Some(start.day()),
        ..discounted("20")
    };
    let decimal = |s: &str| s.parse::<rust_decimal::Decimal>().unwrap();

    let created = env.create(payload.clone()).await;
    let totals = env.get(&created.quote.unwrap().id).await.totals.unwrap();
    assert_eq!(totals.currency, "EUR");
    assert!(!totals.has_usage);
    assert_eq!(totals.recurring.len(), 1);
    let monthly = &totals.recurring[0];
    assert_eq!(
        monthly.period,
        api::subscriptions::v1::SubscriptionFeeBillingPeriod::Monthly as i32
    );
    // Five seats at 10.00, less 20%.
    assert_eq!(
        decimal(&monthly.amount),
        rust_decimal::Decimal::new(4000, 2)
    );
    assert_eq!(
        decimal(&monthly.list_amount),
        rust_decimal::Decimal::new(5000, 2)
    );
    assert_eq!(
        totals.first_invoice_subtotal.as_deref().map(decimal),
        Some(rust_decimal::Decimal::new(4000, 2))
    );

    let previewed = env
        .clients
        .quotes
        .clone()
        .preview_quote_totals(api::quotes::v1::PreviewQuoteTotalsRequest {
            quote: Some(payload),
        })
        .await
        .unwrap()
        .into_inner()
        .totals
        .unwrap();
    assert_eq!(previewed, totals, "the editor shows what saving stores");

    let quotes = env
        .clients
        .quotes
        .clone()
        .list_quotes(api::quotes::v1::ListQuotesRequest::default())
        .await
        .unwrap()
        .into_inner()
        .quotes;
    assert_eq!(quotes.len(), 1, "previewing stores nothing");
}

#[tokio::test]
async fn sending_records_when_and_to_whom() {
    let env = env().await;
    let created = env
        .create(api::quotes::v1::CreateQuote {
            countersigners: vec![api::quotes::v1::QuoteCountersigner {
                name: "Sam Seller".to_owned(),
                email: "sam@seller.test".to_owned(),
                title: None,
                automatic: false,
            }],
            ..quote()
        })
        .await
        .quote
        .unwrap();
    assert!(created.last_sent_at.is_none());
    let send = |recipient_email: Option<&str>| {
        let mut quotes = env.clients.quotes.clone();
        let request = api::quotes::v1::SendQuoteRequest {
            id: created.id.clone(),
            recipient_email: recipient_email.map(str::to_owned),
            message: None,
            send_copy_to_sender: None,
        };
        async move { quotes.send_quote(request).await }
    };
    let sent_to = || async {
        let evidence = env.evidence(&created.id).await;
        let sent = evidence
            .events
            .iter()
            .rev()
            .find(|e| e.kind == "sent")
            .expect("a send is in the evidence");
        let details: serde_json::Value = serde_json::from_str(&sent.details).unwrap();
        details["recipients"].clone()
    };
    let last_sent_at = || async {
        let at = env
            .get(&created.id)
            .await
            .quote
            .unwrap()
            .last_sent_at
            .unwrap();
        chrono::DateTime::parse_from_rfc3339(&at).unwrap()
    };

    send(None).await.unwrap();
    let first = last_sent_at().await;
    assert_eq!(sent_to().await, serde_json::json!([SIGNER]));

    let unknown = send(Some("someone@else.test")).await.unwrap_err();
    assert_eq!(unknown.code(), Code::InvalidArgument);

    common_clock::test::advance(chrono::Duration::minutes(5));
    send(Some(" SAM@SELLER.TEST ")).await.unwrap();
    assert_eq!(
        sent_to().await,
        serde_json::json!(["sam@seller.test"]),
        "only that person, with the stored spelling"
    );
    assert!(last_sent_at().await > first);
}

#[tokio::test]
async fn a_countersigner_sees_which_line_is_theirs() {
    let env = env().await;
    let created = env
        .create(api::quotes::v1::CreateQuote {
            countersigners: vec![api::quotes::v1::QuoteCountersigner {
                name: "Sam Seller".to_owned(),
                email: "sam@seller.test".to_owned(),
                title: None,
                automatic: false,
            }],
            ..quote()
        })
        .await
        .quote
        .unwrap();
    env.publish(&created.id).await.unwrap();
    let (env, id) = (&env, &created.id);
    let shown_to = move |email: &'static str| async move {
        let token = env.portal_token(id, email).await;
        env.portal()
            .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
            .await
            .unwrap()
            .into_inner()
            .quote
            .unwrap()
            .countersigners
    };

    let to_countersigner = shown_to("SAM@seller.test").await;
    assert!(to_countersigner[0].is_viewer);
    let to_customer = shown_to(SIGNER).await;
    assert!(!to_customer[0].is_viewer);
}

#[tokio::test]
async fn the_portal_takes_a_drawn_or_typed_signature_only() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    env.publish(&created.id).await.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;
    let hash = env.get(&created.id).await.quote.unwrap().document_hash;
    let sign = |method: &str| {
        let mut portal = env.portal();
        let request = with_token(
            portal::SignQuoteRequest {
                recipient_email: SIGNER.to_owned(),
                signed_by_name: "Jane".to_owned(),
                signed_by_title: "CFO".to_owned(),
                signature_data: "data:image/png;base64,AAAA".to_owned(),
                signature_method: method.to_owned(),
                document_hash: hash.clone(),
                consent_text: Some("I accept".to_owned()),
            },
            &token,
        );
        async move { portal.sign_quote(request).await }
    };

    for seller_method in ["countersignature", "automatic_countersignature", ""] {
        let refused = sign(seller_method).await.unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument, "{seller_method}");
    }
    let signed = sign("typed").await.unwrap().into_inner();
    assert!(signed.accepted);
    let method = env
        .get(&created.id)
        .await
        .signatures
        .into_iter()
        .find(|s| s.signed_by_email == SIGNER)
        .unwrap()
        .signature_method;
    assert_eq!(method, "typed");
}

#[tokio::test]
async fn a_quote_is_in_its_plan_s_currency() {
    let env = env().await;
    let in_dollars = api::quotes::v1::CreateQuote {
        currency: "USD".to_owned(),
        ..quote()
    };
    let mut quotes = env.clients.quotes.clone();

    let created = quotes
        .create_quote(api::quotes::v1::CreateQuoteRequest {
            quote: Some(in_dollars.clone()),
        })
        .await
        .unwrap_err();
    assert_eq!(created.code(), Code::InvalidArgument);

    let previewed = quotes
        .preview_quote_totals(api::quotes::v1::PreviewQuoteTotalsRequest {
            quote: Some(in_dollars.clone()),
        })
        .await
        .unwrap_err();
    assert_eq!(previewed.code(), Code::InvalidArgument);

    let existing = env.create(quote()).await.quote.unwrap();
    let updated = quotes
        .update_quote(api::quotes::v1::UpdateQuoteRequest {
            id: existing.id,
            quote: Some(in_dollars),
            expected_updated_at: None,
        })
        .await
        .unwrap_err();
    assert_eq!(updated.code(), Code::InvalidArgument);
}

impl Env {
    async fn verified_quote(&self) -> (String, String) {
        let created = self
            .create(api::quotes::v1::CreateQuote {
                signature_level: api::quotes::v1::QuoteSignatureLevel::Verified.into(),
                ..quote()
            })
            .await
            .quote
            .unwrap();
        self.publish(&created.id).await.unwrap();
        let token = self.portal_token(&created.id, SIGNER).await;
        (created.id, token)
    }
}

#[tokio::test]
async fn the_seller_previews_the_portal_without_opening_it() {
    let env = env().await;
    let created = env.create(quote()).await.quote.unwrap();
    env.publish(&created.id).await.unwrap();
    let token = env.portal_token(&created.id, SIGNER).await;
    env.portal()
        .add_quote_comment(with_token(
            portal::AddQuoteCommentRequest {
                body: "Can the seats start in March?".to_owned(),
                request_changes: false,
                anchor: None,
            },
            &token,
        ))
        .await
        .unwrap();

    let preview = |recipient_email: Option<&str>| {
        let mut quotes = env.clients.quotes.clone();
        let request = api::quotes::v1::PreviewQuotePortalRequest {
            quote_id: created.id.clone(),
            recipient_email: recipient_email.map(str::to_owned),
        };
        async move { quotes.preview_quote_portal(request).await }
    };

    let shown = preview(None).await.unwrap().into_inner().quote.unwrap();
    assert_eq!(shown.current_recipient_email, SIGNER);
    assert_eq!(shown.comments.len(), 1);
    assert_eq!(shown.comments[0].body, "Can the seats start in March?");
    assert!(shown.totals.is_some());

    let unknown = preview(Some("someone@else.test")).await.unwrap_err();
    assert_eq!(unknown.code(), Code::InvalidArgument);

    let engagement = env.get(&created.id).await.engagement.unwrap();
    assert!(engagement.recipients.is_empty(), "a preview is no view");
    assert!(engagement.sessions.is_empty());
    assert!(
        env.evidence(&created.id)
            .await
            .events
            .iter()
            .all(|e| e.kind != "viewed")
    );

    // What the recipient then opens carries the same branding.
    let opened = env
        .portal()
        .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap();
    assert_eq!(opened.theme_mode, shown.theme_mode);
    assert_eq!(opened.roundness, shown.roundness);
    assert_eq!(opened.show_powered_by, shown.show_powered_by);
    assert_eq!(opened.comments, shown.comments);
}

/// The emails a quote's outcome sends, delivered through the email worker.
mod outcome_emails {
    use super::{SEED_USERNAME, quote, with_token};
    use crate::data::ids::INVOICING_ENTITY_ID;
    use crate::harness::{TestEnv, test_env};
    use crate::test_customer_emails::{clients, enable_emails};
    use meteroid_grpc::meteroid::api;
    use meteroid_grpc::meteroid::portal::quotes::v1 as portal;
    use meteroid_grpc::meteroid::portal::quotes::v1::portal_quote_service_client::PortalQuoteServiceClient;
    use meteroid_mailer::model::Email;
    use rstest::rstest;

    const BUYER: &str = "buyer@spotify.example";
    const LEGAL: &str = "legal@spotify.example";

    fn signers(emails: &[&str]) -> api::quotes::v1::CreateQuote {
        api::quotes::v1::CreateQuote {
            recipients: emails
                .iter()
                .map(|email| api::quotes::v1::RecipientDetails {
                    name: "Signer".to_owned(),
                    email: (*email).to_owned(),
                })
                .collect(),
            ..quote()
        }
    }

    async fn published(env: &TestEnv, quote: api::quotes::v1::CreateQuote) -> String {
        let mut quotes = clients(env).await.quotes;
        let id = quotes
            .create_quote(api::quotes::v1::CreateQuoteRequest { quote: Some(quote) })
            .await
            .unwrap()
            .into_inner()
            .quote
            .unwrap()
            .quote
            .unwrap()
            .id;
        quotes
            .publish_quote(api::quotes::v1::PublishQuoteRequest { id: id.clone() })
            .await
            .unwrap();
        id
    }

    async fn token(env: &TestEnv, id: &str, email: &str) -> String {
        clients(env)
            .await
            .quotes
            .generate_quote_portal_token(api::quotes::v1::GenerateQuotePortalTokenRequest {
                quote_id: id.to_owned(),
                recipient_email: email.to_owned(),
            })
            .await
            .unwrap()
            .into_inner()
            .token
    }

    async fn sign_as(env: &TestEnv, id: &str, email: &str) -> portal::SignQuoteResponse {
        let hash = clients(env)
            .await
            .quotes
            .get_quote(api::quotes::v1::GetQuoteRequest { id: id.to_owned() })
            .await
            .unwrap()
            .into_inner()
            .quote
            .unwrap()
            .quote
            .unwrap()
            .document_hash;
        let token = token(env, id, email).await;
        PortalQuoteServiceClient::new(env.setup.channel.clone())
            .sign_quote(with_token(
                portal::SignQuoteRequest {
                    recipient_email: email.to_owned(),
                    signed_by_name: "Signer".to_owned(),
                    signed_by_title: String::new(),
                    signature_data: "data:image/png;base64,AAAA".to_owned(),
                    signature_method: "drawn".to_owned(),
                    document_hash: hash,
                    consent_text: Some("I accept".to_owned()),
                },
                &token,
            ))
            .await
            .unwrap()
            .into_inner()
    }

    fn to(email: &Email) -> Vec<String> {
        email.to.iter().map(|r| r.email.clone()).collect()
    }

    // The seeded invoicing entity writes in French, to its customers and to its team.
    const ACCEPTED_SUBJECT_END: &str = "est accepté";

    /// The customer emails telling a quote is accepted, by recipient.
    fn accepted_to_customers(sent: &[Email]) -> Vec<Vec<String>> {
        let mut found: Vec<Vec<String>> = sent
            .iter()
            .filter(|e| e.subject.ends_with(ACCEPTED_SUBJECT_END))
            .map(to)
            .collect();
        found.sort();
        found
    }

    fn to_owner<'a>(sent: &'a [Email], outcome: &str) -> Vec<&'a Email> {
        sent.iter()
            .filter(|e| to(e) == vec![SEED_USERNAME.to_owned()])
            .filter(|e| e.subject.contains(&format!("{outcome} le devis")))
            .collect()
    }

    #[rstest]
    #[tokio::test]
    async fn acceptance_emails_each_recipient_and_the_owner(#[future] test_env: TestEnv) {
        let env = test_env.await;
        enable_emails(&env).await;
        let id = published(&env, signers(&[BUYER, LEGAL])).await;

        assert!(!sign_as(&env, &id, BUYER).await.accepted);
        let sent = env.drain_email_queue().await;
        assert!(accepted_to_customers(&sent).is_empty(), "not accepted yet");
        assert!(to_owner(&sent, "a accepté").is_empty());

        assert!(sign_as(&env, &id, LEGAL).await.accepted);
        let sent = env.drain_email_queue().await;
        assert_eq!(
            accepted_to_customers(&sent),
            vec![vec![BUYER.to_owned()], vec![LEGAL.to_owned()]]
        );
        assert!(
            sent.iter()
                .filter(|e| e.subject.ends_with(ACCEPTED_SUBJECT_END))
                .all(|e| e.body_html.contains("/portal/quote?token=")),
            "each links to the signed quote"
        );
        assert_eq!(to_owner(&sent, "a accepté").len(), 1);

        assert!(env.drain_email_queue().await.is_empty(), "sent once");
    }

    #[rstest]
    #[tokio::test]
    async fn a_countersigned_quote_emails_once_countersigned(#[future] test_env: TestEnv) {
        let env = test_env.await;
        enable_emails(&env).await;
        let id = published(
            &env,
            api::quotes::v1::CreateQuote {
                countersigners: vec![api::quotes::v1::QuoteCountersigner {
                    name: "Demo User".to_owned(),
                    email: SEED_USERNAME.to_owned(),
                    title: None,
                    automatic: false,
                }],
                ..signers(&[BUYER])
            },
        )
        .await;

        let signed = sign_as(&env, &id, BUYER).await;
        assert!(signed.awaiting_countersignature);
        let sent = env.drain_email_queue().await;
        assert!(accepted_to_customers(&sent).is_empty());
        assert!(to_owner(&sent, "a accepté").is_empty());

        let hash = clients(&env)
            .await
            .quotes
            .get_quote(api::quotes::v1::GetQuoteRequest { id: id.clone() })
            .await
            .unwrap()
            .into_inner()
            .quote
            .unwrap()
            .quote
            .unwrap()
            .document_hash
            .unwrap();
        clients(&env)
            .await
            .quotes
            .countersign_quote(api::quotes::v1::CountersignQuoteRequest {
                quote_id: id.clone(),
                signature_data: None,
                title: None,
                document_hash: hash,
                consent_text: Some("Countersigned".to_owned()),
            })
            .await
            .unwrap();
        let sent = env.drain_email_queue().await;
        assert_eq!(accepted_to_customers(&sent), vec![vec![BUYER.to_owned()]]);
        assert_eq!(to_owner(&sent, "a accepté").len(), 1);
    }

    #[rstest]
    #[tokio::test]
    async fn a_disabled_acceptance_email_still_tells_the_owner(#[future] test_env: TestEnv) {
        let env = test_env.await;
        enable_emails(&env).await;
        clients(&env)
            .await
            .customer_emails
            .set_email_kind_enabled(api::customeremails::v1::SetEmailKindEnabledRequest {
                invoicing_entity_id: INVOICING_ENTITY_ID.as_proto(),
                kind: api::customeremails::v1::EmailKind::QuoteAccepted.into(),
                enabled: false,
            })
            .await
            .unwrap();
        let id = published(&env, signers(&[BUYER])).await;

        assert!(sign_as(&env, &id, BUYER).await.accepted);
        let sent = env.drain_email_queue().await;
        assert!(accepted_to_customers(&sent).is_empty());
        assert_eq!(to_owner(&sent, "a accepté").len(), 1);
    }

    #[rstest]
    #[tokio::test]
    async fn a_declined_quote_tells_the_owner_why(#[future] test_env: TestEnv) {
        let env = test_env.await;
        enable_emails(&env).await;
        let id = published(&env, signers(&[BUYER])).await;
        let token = token(&env, &id, BUYER).await;
        PortalQuoteServiceClient::new(env.setup.channel.clone())
            .decline_quote(with_token(
                portal::DeclineQuoteRequest {
                    reason: Some("Over our budget this year".to_owned()),
                },
                &token,
            ))
            .await
            .unwrap();

        let sent = env.drain_email_queue().await;
        let declined = to_owner(&sent, "a refusé");
        assert_eq!(declined.len(), 1);
        assert!(declined[0].body_html.contains("Over our budget this year"));
        assert!(accepted_to_customers(&sent).is_empty());
    }

    #[rstest]
    #[tokio::test]
    async fn a_test_mode_workspace_keeps_the_acceptance_from_customers(
        #[future] test_env: TestEnv,
    ) {
        let env = test_env.await;
        env.set_email_delivery_mode(diesel_models::enums::EmailDeliveryModeEnum::Test)
            .await;
        let id = published(&env, signers(&[BUYER])).await;

        assert!(sign_as(&env, &id, BUYER).await.accepted);
        let sent = env.drain_email_queue().await;
        assert!(
            accepted_to_customers(&sent).is_empty(),
            "a test workspace emails only its team"
        );
        assert_eq!(to_owner(&sent, "a accepté").len(), 1);
    }
}

#[tokio::test]
async fn the_document_states_the_terms_of_its_coupons() {
    let env = env().await;
    let coupon = |code: &str,
                  description: &str,
                  discount: api::coupons::v1::coupon_discount::DiscountType,
                  recurring_value: Option<i32>| {
        let mut coupons = env.clients.coupons.clone();
        let request = api::coupons::v1::CreateCouponRequest {
            code: code.to_owned(),
            description: description.to_owned(),
            discount: Some(api::coupons::v1::CouponDiscount {
                discount_type: Some(discount),
            }),
            expires_at: None,
            redemption_limit: None,
            recurring_value,
            reusable: false,
            plan_ids: vec![],
        };
        async move {
            coupons
                .create_coupon(request)
                .await
                .unwrap()
                .into_inner()
                .coupon
                .unwrap()
                .id
        }
    };
    let welcome = coupon(
        "WELCOME20",
        "Launch offer",
        api::coupons::v1::coupon_discount::DiscountType::Percentage(
            api::coupons::v1::coupon_discount::PercentageDiscount {
                percentage: "20".to_owned(),
            },
        ),
        Some(3),
    )
    .await;
    let setup = coupon(
        "SETUP100",
        "",
        api::coupons::v1::coupon_discount::DiscountType::Fixed(
            api::coupons::v1::coupon_discount::FixedDiscount {
                amount: "100".to_owned(),
                currency: "EUR".to_owned(),
            },
        ),
        None,
    )
    .await;

    let created = env
        .create(api::quotes::v1::CreateQuote {
            coupons: Some(api::quotes::v1::CreateQuoteCoupons {
                coupons: vec![
                    api::quotes::v1::CreateQuoteCoupon { coupon_id: welcome },
                    api::quotes::v1::CreateQuoteCoupon { coupon_id: setup },
                ],
            }),
            ..quote()
        })
        .await
        .quote
        .unwrap();
    env.publish(&created.id).await.unwrap();

    let expected = vec![
        api::quotes::v1::QuoteCouponTerms {
            code: "WELCOME20".to_owned(),
            description: Some("Launch offer".to_owned()),
            discount: Some(api::coupons::v1::CouponDiscount {
                discount_type: Some(api::coupons::v1::coupon_discount::DiscountType::Percentage(
                    api::coupons::v1::coupon_discount::PercentageDiscount {
                        percentage: "20".to_owned(),
                    },
                )),
            }),
            recurring_value: Some(3),
        },
        api::quotes::v1::QuoteCouponTerms {
            code: "SETUP100".to_owned(),
            description: None,
            discount: Some(api::coupons::v1::CouponDiscount {
                discount_type: Some(api::coupons::v1::coupon_discount::DiscountType::Fixed(
                    api::coupons::v1::coupon_discount::FixedDiscount {
                        amount: "100".to_owned(),
                        currency: "EUR".to_owned(),
                    },
                )),
            }),
            recurring_value: None,
        },
    ];

    let detailed = env.get(&created.id).await;
    assert_eq!(detailed.coupons.len(), 2);
    assert_eq!(detailed.coupon_terms, expected);

    let token = env.portal_token(&created.id, SIGNER).await;
    let shown = env
        .portal()
        .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap();
    assert_eq!(shown.coupon_terms, expected);

    let revisions = env
        .clients
        .quotes
        .clone()
        .list_quote_revisions(api::quotes::v1::ListQuoteRevisionsRequest {
            quote_id: created.id.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .revisions;
    let document: serde_json::Value = serde_json::from_str(&revisions[0].document).unwrap();
    let codes: Vec<&str> = document["coupons"]
        .as_array()
        .expect("the signed document holds the coupon terms")
        .iter()
        .filter_map(|c| c["code"].as_str())
        .collect();
    assert_eq!(codes, ["WELCOME20", "SETUP100"]);
    assert_eq!(document["coupons"][0]["recurring_value"], 3);
}

#[tokio::test]
async fn a_coupon_discount_is_locked_while_an_open_quote_offers_it() {
    let env = env().await;
    let percent = |percentage: &str| api::coupons::v1::CouponDiscount {
        discount_type: Some(api::coupons::v1::coupon_discount::DiscountType::Percentage(
            api::coupons::v1::coupon_discount::PercentageDiscount {
                percentage: percentage.to_owned(),
            },
        )),
    };
    let coupon_id = env
        .clients
        .coupons
        .clone()
        .create_coupon(api::coupons::v1::CreateCouponRequest {
            code: "LAUNCH20".to_owned(),
            description: "Launch offer".to_owned(),
            discount: Some(percent("20")),
            expires_at: None,
            redemption_limit: None,
            recurring_value: None,
            reusable: true,
            plan_ids: vec![],
        })
        .await
        .unwrap()
        .into_inner()
        .coupon
        .unwrap()
        .id;
    let edit = |description: &str, percentage: &str| {
        let mut coupons = env.clients.coupons.clone();
        let request = api::coupons::v1::EditCouponRequest {
            coupon_id: coupon_id.clone(),
            description: description.to_owned(),
            discount: Some(percent(percentage)),
            plan_ids: vec![],
        };
        async move { coupons.edit_coupon(request).await.map(|_| ()) }
    };
    let offered = || api::quotes::v1::CreateQuote {
        coupons: Some(api::quotes::v1::CreateQuoteCoupons {
            coupons: vec![api::quotes::v1::CreateQuoteCoupon {
                coupon_id: coupon_id.clone(),
            }],
        }),
        ..quote()
    };

    let draft = env.create(offered()).await.quote.unwrap();
    edit("Launch offer", "25").await.unwrap();
    edit("Launch offer", "20").await.unwrap();

    let signed = env.create(offered()).await.quote.unwrap();
    let cancelled = env.create(offered()).await.quote.unwrap();
    env.publish(&signed.id).await.unwrap();
    env.publish(&cancelled.id).await.unwrap();

    let refused = edit("Launch offer", "10").await.unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(refused.message().contains(&signed.quote_number));
    assert!(refused.message().contains(&cancelled.quote_number));
    assert!(!refused.message().contains(&draft.quote_number));
    edit("Spring offer", "20").await.unwrap();
    edit("Spring offer", "20.00").await.unwrap();

    let token = env.portal_token(&signed.id, SIGNER).await;
    let hash = env
        .portal()
        .get_quote_portal(with_token(portal::GetQuotePortalRequest {}, &token))
        .await
        .unwrap()
        .into_inner()
        .quote
        .unwrap()
        .quote
        .unwrap()
        .document_hash;
    assert!(env.sign(&token, hash).await.unwrap().accepted);
    env.clients
        .quotes
        .clone()
        .cancel_quote(api::quotes::v1::CancelQuoteRequest {
            id: cancelled.id.clone(),
            reason: None,
        })
        .await
        .unwrap();
    let accepted = edit("Spring offer", "10").await.unwrap_err();
    assert_eq!(accepted.code(), Code::InvalidArgument);
    assert!(accepted.message().contains(&signed.quote_number));
    assert!(!accepted.message().contains(&cancelled.quote_number));

    env.clients
        .quotes
        .clone()
        .convert_quote_to_subscription(api::quotes::v1::ConvertQuoteToSubscriptionRequest {
            quote_id: signed.id.clone(),
        })
        .await
        .unwrap();
    edit("Spring offer", "10").await.unwrap();
}
