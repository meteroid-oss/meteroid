//! What credit cost: every grant records the money behind it, and every ledger movement the share
//! of that cost it carried, split cumulatively so a grant's entries sum exactly to its cost.
use crate::data::ids::{CUST_UBER_ID, TENANT_ID};
use crate::helpers::credit::{Grant, expire_grant, fund_promotional_grant};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use crate::test_customer_credit::{draft, finalize, line, pay_amount};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, CreditGrantId};
use diesel::sql_types::{BigInt, Nullable};
use diesel_async::RunQueryDsl;
use meteroid_store::domain::credit::{
    CreditCost, CreditGrant, CreditGrantSource, GrantClassification, LedgerEntryKind,
};
use meteroid_store::domain::credit_units::{CreditUnitNew, CreditUnitRef};
use meteroid_store::domain::price_components::CreditExpiry;
use meteroid_store::domain::{Invoice, LineItem, LineItemKind};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::{CreditNoteInterface, CreditUnitsInterface, InvoiceInterface};
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    crate::meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await
}

async fn customer_currency(setup: &MeteroidSetup) -> TestResult<String> {
    Ok(setup
        .store
        .find_customer_by_id(CUST_UBER_ID, TENANT_ID)
        .await
        .map_err(test_error)?
        .currency)
}

/// A pack selling `credit` of credit for a line of `subtotal`, discounted to `net`.
fn pack(credit: i64, subtotal: i64, net: i64, expiry: Option<CreditExpiry>) -> LineItem {
    LineItem {
        kind: LineItemKind::Credit,
        credit_grant_amount: Some(credit),
        credit_grant_expiry: expiry,
        amount_subtotal: subtotal,
        taxable_amount: net,
        amount_total: net,
        ..line("Credit pack", subtotal, false)
    }
}

async fn bought(setup: &MeteroidSetup, pack: LineItem) -> TestResult<Invoice> {
    let invoice = draft(setup, vec![pack]).await?;
    let invoice = finalize(setup, invoice.id).await?.invoice;
    pay_amount(setup, invoice.id, invoice.amount_due).await?;
    Ok(invoice)
}

/// Finalizes an invoice of `cents` that customer credit pays.
async fn spend(setup: &MeteroidSetup, cents: i64) -> TestResult {
    let invoice = draft(setup, vec![line("Seats", cents, false)]).await?;
    let finalized = finalize(setup, invoice.id).await?.invoice;
    assert_eq!(
        finalized.applied_credits + finalized.promotional_credits,
        cents,
        "credit pays the whole invoice"
    );
    Ok(())
}

async fn grants(setup: &MeteroidSetup, source: CreditGrantSource) -> TestResult<Vec<CreditGrant>> {
    Ok(setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .into_iter()
        .flat_map(|wallet| wallet.grants)
        .filter(|grant| grant.source == source)
        .collect())
}

/// Costs of one grant's statement lines of `kind`, oldest first.
async fn entry_costs(
    setup: &MeteroidSetup,
    grant: CreditGrantId,
    kind: LedgerEntryKind,
) -> TestResult<Vec<i64>> {
    let mut costs = Vec::new();
    for wallet in setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
    {
        let (entries, _) = setup
            .store
            .customer_credit_ledger(TENANT_ID, wallet.id, 1000, 0)
            .await
            .map_err(test_error)?;
        for entry in entries.iter().rev() {
            if entry.grant_id == grant && entry.kind == kind {
                costs.push(
                    entry
                        .cost
                        .as_ref()
                        .ok_or("an entry of a costed grant has no cost")?
                        .amount_minor,
                );
            }
        }
    }
    Ok(costs)
}

#[derive(diesel::QueryableByName)]
struct GrantCosts {
    #[diesel(sql_type = BigInt)]
    amount: i64,
    #[diesel(sql_type = BigInt)]
    remaining: i64,
    #[diesel(sql_type = Nullable<BigInt>)]
    cost: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    entries: Option<i64>,
    #[diesel(sql_type = BigInt)]
    uncosted: i64,
}

/// Every costed grant's entries hold exactly the cost of what it still holds, rounded once, and
/// an uncosted grant's entries carry none.
async fn assert_costs_conserved(setup: &MeteroidSetup) -> TestResult {
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let rows: Vec<GrantCosts> = diesel::sql_query(format!(
        "SELECT g.amount, g.remaining, g.cost_minor AS cost, sum(e.cost_minor)::bigint AS entries,
                count(*) FILTER (WHERE (e.cost_minor IS NULL) <> (g.cost_minor IS NULL)) AS uncosted
         FROM customer_credit_grant g JOIN customer_credit_entry e ON e.grant_id = g.id
         WHERE g.tenant_id = '{}' GROUP BY g.id",
        TENANT_ID.as_uuid()
    ))
    .load(&mut conn)
    .await
    .map_err(test_error)?;
    assert!(!rows.is_empty());
    for row in rows {
        assert_eq!(
            row.uncosted, 0,
            "an entry's cost is set exactly when its grant's is"
        );
        let Some(cost) = row.cost else { continue };
        let consumed = i128::from(row.amount - row.remaining);
        let consumed_cost = (2 * consumed * i128::from(cost) + i128::from(row.amount))
            / (2 * i128::from(row.amount));
        assert_eq!(
            row.entries.map(i128::from),
            Some(i128::from(cost) - consumed_cost),
            "a grant's entries sum to the cost of what it still holds"
        );
    }
    Ok(())
}

/// The paid grant of a purchase and the bonus credit sold with it.
async fn paid_and_bonus(setup: &MeteroidSetup) -> TestResult<(CreditGrant, CreditGrant)> {
    let bought = grants(setup, CreditGrantSource::Purchase).await?;
    let of = |classification| {
        bought
            .iter()
            .find(|grant| grant.classification == classification)
            .cloned()
            .ok_or("a purchased grant")
    };
    Ok((
        of(GrantClassification::Paid)?,
        of(GrantClassification::Promotional)?,
    ))
}

/// A pack of 1,000 discounted to 800 is 800 of paid credit at its cost and 200 of bonus credit
/// at none. Three odd draws, the bonus first, carry all the cost: what was paid.
#[tokio::test]
async fn a_discounted_pack_costs_its_net_amount_and_its_draws_carry_all_of_it() -> TestResult {
    let setup = setup().await;
    let currency = customer_currency(&setup).await?;
    bought(&setup, pack(1_000, 1_000, 800, None)).await?;
    let (grant, bonus) = paid_and_bonus(&setup).await?;
    assert_eq!(grant.amount_minor, 800);
    assert_eq!(grant.cost, Some(CreditCost::at_face(800, &currency)));
    assert_eq!(bonus.amount_minor, 200);
    assert_eq!(bonus.cost, Some(CreditCost::given(&currency)));
    assert_eq!(
        entry_costs(&setup, grant.id, LedgerEntryKind::Fund).await?,
        vec![800]
    );

    for cents in [333, 333, 334] {
        spend(&setup, cents).await?;
        assert_costs_conserved(&setup).await?;
    }
    assert_eq!(
        entry_costs(&setup, bonus.id, LedgerEntryKind::Draw).await?,
        vec![0]
    );
    let draws = entry_costs(&setup, grant.id, LedgerEntryKind::Draw).await?;
    assert_eq!(draws, vec![-133, -333, -334]);
    assert_eq!(
        draws.iter().sum::<i64>(),
        -800,
        "recognized revenue is the whole cost"
    );
    Ok(())
}

/// Unspent credit lapses with the cost it still holds: breakage, never more than was paid.
#[tokio::test]
async fn an_expired_pack_records_its_unconsumed_cost_as_breakage() -> TestResult {
    let setup = setup().await;
    setup.skip_credit_invariants(
        "moves a grant's expiry or a subscription's end into the past by raw SQL after credit was drawn",
    );
    bought(&setup, pack(1_000, 1_000, 800, Some(CreditExpiry::Day(30)))).await?;
    let (grant, _) = paid_and_bonus(&setup).await?;
    spend(&setup, 333).await?;
    expire_grant(&setup.store, grant.id).await;
    assert_eq!(
        entry_costs(&setup, grant.id, LedgerEntryKind::Expiry).await?,
        vec![-667],
        "the statement shows the lapse before the sweep records it"
    );
    assert_eq!(
        setup
            .store
            .expire_customer_credit(200)
            .await
            .map_err(test_error)?,
        1
    );
    assert_eq!(
        entry_costs(&setup, grant.id, LedgerEntryKind::Draw).await?,
        vec![-133]
    );
    assert_eq!(
        entry_costs(&setup, grant.id, LedgerEntryKind::Expiry).await?,
        vec![-667]
    );
    assert_costs_conserved(&setup).await?;
    Ok(())
}

/// A pack of 10 sold for 30 and refunded in three notes takes 3, 7 and 10 back: its cost is
/// refunded cumulatively, 9, 12 and 9.
#[tokio::test]
async fn refunds_of_a_pack_reverse_its_cost_cumulatively() -> TestResult {
    let setup = setup().await;
    setup.skip_credit_invariants(
        "sells a currency pack above its face through store internals, which no API allows",
    );
    let invoice = bought(&setup, pack(10, 30, 30, None)).await?;
    let grant = grants(&setup, CreditGrantSource::Purchase).await?.remove(0);
    for _ in 0..3 {
        setup
            .store
            .create_and_finalize_credit_note(
                Actor::System,
                TENANT_ID,
                CreateCreditNoteParams {
                    invoice_id: invoice.id,
                    line_items: vec![CreditLineItem::Amount {
                        local_id: invoice.line_items[0].local_id.clone(),
                        amount_cents: 10,
                    }],
                    reason: Some("Refund".into()),
                    memo: None,
                    credit_type: CreditType::Refund,
                    custom_properties: None,
                },
            )
            .await
            .map_err(test_error)?;
        assert_costs_conserved(&setup).await?;
    }
    assert_eq!(
        entry_costs(&setup, grant.id, LedgerEntryKind::Reversal).await?,
        vec![-9, -12, -9]
    );
    Ok(())
}

/// Given credit costs nothing; credit that is the customer's money costs its face.
#[tokio::test]
async fn given_credit_costs_nothing_and_the_customers_own_money_its_face() -> TestResult {
    let setup = setup().await;
    let currency = customer_currency(&setup).await?;
    let promotional = fund_promotional_grant(
        &setup.store,
        TENANT_ID,
        CUST_UBER_ID,
        None,
        Grant::invoice(1_500),
    )
    .await;
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, &currency)
        .await
        .map_err(test_error)?;
    setup
        .store
        .fund_customer_credit_wallet(FundCustomerCreditWallet {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            wallet_id: wallet.id,
            amount_minor: 700,
            classification: GrantClassification::Paid,
            priority: 100,
            expires_at: None,
            idempotency_key: Uuid::now_v7(),
            reason: "Paid outside Meteroid".to_owned(),
            reference: None,
            actor_id: None,
            cost: None,
        })
        .await
        .map_err(test_error)?;
    let negative = draft(&setup, vec![line("Downgrade proration", -2_500, false)]).await?;
    finalize(&setup, negative.id).await?;

    let manual = grants(&setup, CreditGrantSource::Manual).await?;
    let cost_of = |id: CreditGrantId| {
        manual
            .iter()
            .find(|g| g.id == id)
            .and_then(|g| g.cost.clone())
    };
    assert_eq!(cost_of(promotional), Some(CreditCost::given(&currency)));
    let paid = manual
        .iter()
        .find(|g| g.id != promotional)
        .ok_or("paid grant")?;
    assert_eq!(paid.cost, Some(CreditCost::at_face(700, &currency)));
    assert_eq!(
        grants(&setup, CreditGrantSource::NegativeInvoice).await?[0].cost,
        Some(CreditCost::at_face(2_500, &currency))
    );

    // Promotional credit burns first at equal priority: its draws carry nothing.
    spend(&setup, 1_600).await?;
    assert_eq!(
        entry_costs(&setup, promotional, LedgerEntryKind::Draw).await?,
        vec![0]
    );
    assert_costs_conserved(&setup).await?;
    Ok(())
}

/// A pack of 100 CREDITS (precision 0) sold for 10.00: what it grants is counted in credits, its
/// price in money.
async fn unit_pack(setup: &MeteroidSetup) -> TestResult<LineItem> {
    let unit = setup
        .store
        .create_credit_unit(
            CreditUnitNew::new(TENANT_ID, "CREDITS", "Credit", "Credits", None, 0)
                .map_err(test_error)?,
        )
        .await
        .map_err(test_error)?;
    Ok(LineItem {
        taxed_at_purchase: true,
        credit_grant_unit: Some(CreditUnitRef {
            unit_id: unit.id,
            code: "CREDITS".to_owned(),
            precision: 0,
            product_id: None,
        }),
        ..pack(100, 1_000, 1_000, None)
    })
}

fn half_of(invoice: &Invoice, credit_type: CreditType) -> CreateCreditNoteParams {
    CreateCreditNoteParams {
        invoice_id: invoice.id,
        line_items: vec![CreditLineItem::Amount {
            local_id: invoice.line_items[0].local_id.clone(),
            amount_cents: 500,
        }],
        reason: Some("Half".into()),
        memo: None,
        credit_type,
        custom_properties: None,
    }
}

/// Refunding half the money takes back half the credits with half the cost: 50 credits and
/// 5.00, never the 500 minor units of money read as credits (which reversed the whole pack).
#[tokio::test]
async fn a_partial_refund_of_a_unit_pack_reverses_half_its_credits_and_cost() -> TestResult {
    let setup = setup().await;
    let invoice = bought(&setup, unit_pack(&setup).await?).await?;
    let grant = grants(&setup, CreditGrantSource::Purchase).await?.remove(0);
    assert_eq!(
        (
            grant.amount_minor,
            grant.cost.as_ref().map(|c| c.amount_minor)
        ),
        (100, Some(1_000))
    );

    setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            half_of(&invoice, CreditType::Refund),
        )
        .await
        .map_err(test_error)?;
    let grant = grants(&setup, CreditGrantSource::Purchase).await?.remove(0);
    assert_eq!(grant.remaining_minor, 50);
    assert_eq!(
        entry_costs(&setup, grant.id, LedgerEntryKind::Reversal).await?,
        vec![-500]
    );
    assert_costs_conserved(&setup).await?;
    Ok(())
}

/// Half the debt cancelled before payment: paying the rest funds the 50 credits paid for, at 5.00.
#[tokio::test]
async fn a_unit_pack_half_cancelled_before_payment_funds_half_its_credits() -> TestResult {
    let setup = setup().await;
    let pack = unit_pack(&setup).await?;
    let invoice = draft(&setup, vec![pack]).await?;
    let invoice = finalize(&setup, invoice.id).await?.invoice;
    setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            half_of(&invoice, CreditType::DebtCancellation),
        )
        .await
        .map_err(test_error)?;
    let due = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?
        .invoice
        .amount_due;
    assert!(due > 0 && due < invoice.amount_due);
    pay_amount(&setup, invoice.id, due).await?;

    let grant = grants(&setup, CreditGrantSource::Purchase).await?.remove(0);
    assert_eq!(
        (
            grant.amount_minor,
            grant.cost.as_ref().map(|c| c.amount_minor)
        ),
        (50, Some(500))
    );
    Ok(())
}
