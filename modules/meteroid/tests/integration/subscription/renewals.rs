//! Renewal notices (R-157, R-160, R-161) and the late opt-out waiver (R-152). Starter renews on the
//! 1st, monthly: its periods are never renewals, only its contract term is.

use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, OrganizationId, SubscriptionId, TenantId};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel::QueryableByName;
use diesel::sql_types::{BigInt, Text, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use meteroid_store::domain::outbox_event::OutboxEvent;
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::renewals::{NoticeWaiver, RenewalMode, RenewalSource};
use meteroid_store::domain::subscription_phases::{SubscriptionTerm, SubscriptionTermInput};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::services::renewals::{self, RetentionGate, SweepReport};
use rstest::rstest;

use super::phases::{at, day};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

/// A tenant without retention: only term renewals are noticed.
struct NoRetention;

#[async_trait::async_trait]
impl RetentionGate for NoRetention {
    async fn open(&self, _: TenantId, _: OrganizationId) -> bool {
        false
    }
}

async fn starter(env: &TestEnv) -> SubscriptionId {
    subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

fn term(term_end: NaiveDate, auto_renew: bool, notice_days: u32) -> SubscriptionTermInput {
    SubscriptionTermInput {
        term_end,
        auto_renew,
        renewal_months: 2,
        uplift_percent: None,
        notice_days,
    }
}

async fn set_term(
    env: &TestEnv,
    sub_id: SubscriptionId,
    input: SubscriptionTermInput,
) -> meteroid_store::StoreResult<SubscriptionTerm> {
    env.services()
        .set_subscription_term(Actor::System, TENANT_ID, sub_id, input)
        .await
}

async fn sweep(env: &TestEnv, today: NaiveDate) -> SweepReport {
    renewals::sweep(env.store(), &NoRetention, today)
        .await
        .expect("sweep")
}

/// Each `subscription.renewal_upcoming` queued: its checkpoint, mode, source, date and deadline.
async fn notices(
    env: &TestEnv,
) -> Vec<(
    u32,
    RenewalMode,
    RenewalSource,
    NaiveDate,
    Option<NaiveDate>,
)> {
    env.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .expect("read webhook_out")
        .into_iter()
        .filter_map(|message| message.message)
        .filter_map(|message| serde_json::from_value::<OutboxEvent>(message.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::SubscriptionRenewalUpcoming(e) => Some((
                e.days_before,
                e.mode,
                e.source,
                e.renewal_date,
                e.opt_out_deadline,
            )),
            _ => None,
        })
        .collect()
}

async fn reminders_queued(env: &TestEnv) -> usize {
    env.store()
        .pgmq_read(
            PgmqQueue::SendEmailRequest,
            MessageReadQty(1000),
            MessageReadVtSec(0),
        )
        .await
        .expect("read send_email_request")
        .into_iter()
        .filter_map(|message| message.message)
        .filter(|message| message.0.to_string().contains("renewal_reminder"))
        .count()
}

/// Renewing on May 1 with ten days' notice, checkpoints count to the April 21 deadline: 90, 60
/// and 30 days before it, once each, whichever replica sweeps. Rolled on unwritten to July 1, the
/// next term is noticed from the nearest checkpoint not past.
/// Rules: R-157, R-160
#[rstest]
#[tokio::test]
async fn a_renewing_term_is_noticed_at_each_checkpoint_before_its_deadline(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter(&env).await;
    set_term(&env, sub_id, term(day(5, 1), true, 10))
        .await
        .expect("renewing on May 1");

    assert_eq!(sweep(&env, day(1, 21)).await.team, 0);
    let (a, b) = tokio::join!(
        renewals::sweep(env.store(), &NoRetention, day(1, 22)),
        renewals::sweep(env.store(), &NoRetention, day(1, 22)),
    );
    assert_eq!(a.expect("sweep").team + b.expect("sweep").team, 1);
    assert_eq!(sweep(&env, day(2, 1)).await.team, 0);
    assert_eq!(sweep(&env, day(2, 21)).await.team, 1);
    assert_eq!(sweep(&env, day(3, 22)).await.team, 1);
    assert_eq!(sweep(&env, day(4, 25)).await.team, 0);
    assert_eq!(sweep(&env, day(5, 2)).await.team, 1);
    let (renewing, source, deadline) = (RenewalMode::Renewing, RenewalSource::Term, day(4, 21));
    assert_eq!(
        notices(&env).await,
        vec![
            (90, renewing, source, day(5, 1), Some(deadline)),
            (60, renewing, source, day(5, 1), Some(deadline)),
            (30, renewing, source, day(5, 1), Some(deadline)),
            (60, renewing, source, day(7, 1), Some(day(6, 21))),
        ]
    );
}

/// A term opted out of before its deadline is noticed as ending, counted to its end; a monthly
/// subscription without a term is never noticed.
/// Rules: R-157, R-160
#[rstest]
#[tokio::test]
async fn an_opt_out_rearms_the_notices_as_an_ending(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter(&env).await;
    let other = starter(&env).await;
    set_term(&env, sub_id, term(day(5, 1), true, 10))
        .await
        .expect("renewing on May 1");
    assert_eq!(sweep(&env, day(1, 22)).await.team, 1);
    set_term(&env, sub_id, term(day(5, 1), false, 10))
        .await
        .expect("opted out before the deadline");
    assert_eq!(
        sweep(&env, day(1, 22)).await.team,
        0,
        "ending: 100 days left"
    );
    assert_eq!(sweep(&env, day(2, 1)).await.team, 1, "ending: 90 days left");
    let sent = notices(&env).await;
    assert_eq!(
        sent.last(),
        Some(&(
            90,
            RenewalMode::Ending,
            RenewalSource::Term,
            day(5, 1),
            None
        ))
    );
    let upcoming = renewals::list_upcoming(env.store(), TENANT_ID, 365, day(2, 1), true)
        .await
        .expect("upcoming");
    assert!(upcoming.iter().all(|r| r.subscription_id != other));
}

/// A term created once its deadline has passed gets one notice, at the smallest checkpoint, and
/// no customer reminder; a lead set before the deadline emails the customer once.
/// Rules: R-157, R-161
#[rstest]
#[tokio::test]
async fn a_deadline_already_past_sends_one_notice_and_no_reminder(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let late = starter(&env).await;
    let early = starter(&env).await;
    renewals::update_settings(env.store(), TENANT_ID, Some(vec![30]), Some(Some(30)))
        .await
        .expect("one checkpoint, and a customer reminder, 30 days ahead");
    env.set_now(at(3, 15, 12));
    let yearly = SubscriptionTermInput {
        renewal_months: 12,
        ..term(day(4, 1), true, 30)
    };
    set_term(&env, late, yearly)
        .await
        .expect("deadline March 2, already past");
    set_term(&env, early, term(day(6, 1), true, 30))
        .await
        .expect("deadline May 2");

    assert_eq!(
        sweep(&env, day(3, 15)).await,
        SweepReport {
            team: 1,
            customer: 0
        }
    );
    assert_eq!(
        sweep(&env, day(4, 2)).await,
        SweepReport {
            team: 1,
            customer: 1
        }
    );
    assert_eq!(sweep(&env, day(4, 3)).await, SweepReport::default());
    let renewing = RenewalMode::Renewing;
    assert_eq!(
        notices(&env).await,
        vec![
            (
                30,
                renewing,
                RenewalSource::Term,
                day(4, 1),
                Some(day(3, 2))
            ),
            (
                30,
                renewing,
                RenewalSource::Term,
                day(6, 1),
                Some(day(5, 2))
            ),
        ]
    );
    assert_eq!(reminders_queued(&env).await, 1);
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

/// After its deadline, an opt-out needs the operator's waiver: refused without one, recorded with
/// its reason when given, and refused when there is nothing to waive.
/// Rules: R-152
#[rstest]
#[tokio::test]
async fn a_late_opt_out_needs_a_waiver(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter(&env).await;
    set_term(&env, sub_id, term(day(5, 1), true, 10))
        .await
        .expect("renewing on May 1");
    let waiver = || NoticeWaiver {
        reason: "Agreed with the customer's CFO".to_string(),
    };
    let refused = env
        .services()
        .set_subscription_term_waiving_notice(
            Actor::System,
            TENANT_ID,
            sub_id,
            term(day(5, 1), false, 10),
            waiver(),
        )
        .await
        .expect_err("nothing to waive before the deadline");
    assert!(matches!(
        refused.current_context(),
        StoreError::InvalidArgument(_)
    ));

    env.set_now(at(4, 25, 12));
    let refused = set_term(&env, sub_id, term(day(5, 1), false, 10))
        .await
        .expect_err("past the April 21 deadline");
    assert!(matches!(refused.current_context(), StoreError::Conflict(_)));

    let term = env
        .services()
        .set_subscription_term_waiving_notice(
            Actor::System,
            TENANT_ID,
            sub_id,
            term(day(5, 1), false, 10),
            waiver(),
        )
        .await
        .expect("a late opt-out, waived");
    assert!(!term.auto_renew);
    let mut conn = env.conn().await;
    let waived = diesel::sql_query(
        "SELECT count(*) AS n FROM entity_activity
         WHERE entity_id = $1 AND metadata->'notice_waived'->>'reason' = $2",
    )
    .bind::<SqlUuid, _>(sub_id.as_uuid())
    .bind::<Text, _>("Agreed with the customer's CFO")
    .get_result::<Count>(&mut conn)
    .await
    .expect("activity");
    assert_eq!(waived.n, 1);
    assert_eq!(sweep(&env, day(4, 25)).await.team, 1, "now ending on May 1");
}

/// Cancelled before its term ends, a subscription neither renews nor ends with its term: its
/// cancellation dates it, a source only retention notices.
/// Rules: R-160
#[rstest]
#[tokio::test]
async fn a_cancellation_before_the_term_end_supersedes_it(#[future] test_env: TestEnv) {
    use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
    let env = test_env.await;
    env.set_now(at(1, 15, 12));
    let sub_id = starter(&env).await;
    set_term(&env, sub_id, term(day(5, 1), true, 10))
        .await
        .expect("renewing on May 1");
    env.services()
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(day(4, 1)),
        )
        .await
        .expect("cancelled from April");
    assert_eq!(sweep(&env, day(3, 22)).await, SweepReport::default());
    let upcoming = renewals::list_upcoming(env.store(), TENANT_ID, 365, day(3, 22), true)
        .await
        .expect("upcoming");
    assert_eq!(
        upcoming
            .iter()
            .find(|r| r.subscription_id == sub_id)
            .map(|r| (r.date, r.mode, r.source)),
        Some((
            day(4, 1),
            RenewalMode::Cancelling,
            RenewalSource::Cancellation
        ))
    );
}
