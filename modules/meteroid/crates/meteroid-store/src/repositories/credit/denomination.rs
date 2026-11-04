//! Resolves the `currency` key of a wallet, budget or ledger amount: an ISO currency, otherwise a
//! credit unit of the tenant. Archived units still resolve, their wallets and prices keep them.
use crate::StoreResult;
use crate::domain::credit_units::{CreditUnit, Denomination, DenominationCode};
use crate::services::credit::invalid;
use crate::store::PgConn;
use common_domain::ids::TenantId;
use diesel_models::credit_units::CreditUnitRow;

pub async fn resolve_denomination(
    conn: &mut PgConn,
    tenant_id: TenantId,
    code: &str,
) -> StoreResult<Denomination> {
    match DenominationCode::parse(code)? {
        DenominationCode::Currency(code) => {
            let precision = crate::constants::Currencies::resolve_currency_precision(&code)
                .ok_or_else(|| invalid(format!("unsupported currency {code}")))?;
            Ok(Denomination::Currency { code, precision })
        }
        DenominationCode::Unit(code) => {
            let row = CreditUnitRow::find_by_code(conn, tenant_id, code.as_str())
                .await?
                .ok_or_else(|| invalid(format!("unknown credit unit {code}")))?;
            Ok(Denomination::Unit(CreditUnit::try_from(row)?))
        }
    }
}

/// Minor-unit exponent of a denomination; replaces the ISO-only `scale` once units are funded.
pub async fn denomination_scale(
    conn: &mut PgConn,
    tenant_id: TenantId,
    code: &str,
) -> StoreResult<u8> {
    resolve_denomination(conn, tenant_id, code)
        .await
        .map(|denomination| denomination.precision())
}

/// Decimal places of a credit unit wallet's amounts; None for a currency wallet.
pub async fn account_unit_precision(
    conn: &mut PgConn,
    account: &diesel_models::customer_credit::CustomerCreditAccountRow,
) -> StoreResult<Option<u8>> {
    if account.credit_unit_id.is_none() {
        return Ok(None);
    }
    denomination_scale(conn, account.tenant_id, &account.currency)
        .await
        .map(Some)
}
