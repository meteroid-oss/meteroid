use crate::coupons::{CouponFilter, CouponRow, CouponRowNew, CouponRowPatch, CouponStatusRowPatch};
use crate::errors::IntoDbResult;
use crate::extend::order::{OrderByParam, OrderDirection};
use crate::extend::pagination::{Paginate, PaginatedVec, PaginationRequest};

use crate::{DbResult, PgConn};
use common_domain::ids::{BaseId, CouponId, TenantId};
use diesel::dsl::not;
use diesel::{
    BoolExpressionMethods, ExpressionMethods, NullableExpressionMethods, PgSortExpressionMethods,
    PgTextExpressionMethods, QueryDsl,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use std::collections::HashMap;
use tap::TapFallible;

/// The JSON key a promotional-credit discount is stored under (`CouponDiscount::Credit`).
const CREDIT_DISCOUNT_KEY: &str = "Credit";

/// What a requested coupon code names among candidate rows.
pub enum CodeMatch<'a> {
    Found(&'a CouponRow),
    Unknown,
    /// Several coupons differ from it only in surrounding whitespace or the case of ASCII
    /// letters, none exactly.
    Ambiguous,
}

/// What `str::trim` removes, for SQL's `btrim`: a code stored with surrounding whitespace before
/// codes were validated is matched by the code itself.
const CODE_PADDING: &str = "\t\n\u{0B}\u{0C}\r \u{85}\u{A0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\
    \u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}";

impl<'a> CodeMatch<'a> {
    /// The coupon with exactly that code, as sent then trimmed; else the only one equal to it
    /// once both are trimmed, then ignoring the case of ASCII letters, as Stripe promotion codes
    /// are redeemed.
    pub fn of(code: &str, rows: &'a [CouponRow]) -> Self {
        let trimmed = code.trim();
        if let Some(row) = rows
            .iter()
            .find(|row| row.code == code)
            .or_else(|| rows.iter().find(|row| row.code == trimmed))
        {
            return Self::Found(row);
        }
        Self::only(rows.iter().filter(|row| row.code.trim() == trimmed))
            .or_else(|| {
                Self::only(
                    rows.iter()
                        .filter(|row| row.code.trim().eq_ignore_ascii_case(trimmed)),
                )
            })
            .unwrap_or(Self::Unknown)
    }

    fn only(mut matching: impl Iterator<Item = &'a CouponRow>) -> Option<Self> {
        match (matching.next(), matching.next()) {
            (Some(row), None) => Some(Self::Found(row)),
            (Some(_), Some(_)) => Some(Self::Ambiguous),
            (None, _) => None,
        }
    }
}

impl CouponRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::insert_into(c_dsl::coupon).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting coupon")
            .into_db_result()
    }
}

impl CouponRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CouponId,
    ) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while getting coupon")
            .into_db_result()
    }

    pub async fn list_by_tenant_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        search: Option<String>,
        filter: CouponFilter,
        order_by: Option<&str>,
    ) -> DbResult<PaginatedVec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        let mut query = c_dsl::coupon
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if let Some(search) = search {
            query = query.filter(c_dsl::code.ilike(format!("%{search}%")));
        }

        let is_expired = c_dsl::expires_at
            .is_not_null()
            .and(c_dsl::expires_at.lt(common_domain::clock::now().naive_utc()));

        let is_exhausted = c_dsl::redemption_limit
            .is_not_null()
            .and(c_dsl::redemption_limit.le(c_dsl::redemption_count.nullable()));

        let is_archived = c_dsl::archived_at.is_not_null();

        let is_disabled = c_dsl::disabled.eq(true);

        match filter {
            CouponFilter::ACTIVE => {
                query = query.filter(
                    not(is_expired)
                        .and(not(is_exhausted))
                        .and(not(is_archived))
                        .and(not(is_disabled)),
                );
            }
            CouponFilter::ARCHIVED => {
                query = query.filter(is_archived);
            }
            CouponFilter::INACTIVE => {
                query = query.filter(is_disabled.or(is_expired).or(is_exhausted));
            }
            CouponFilter::ALL => {}
        }

        let order = OrderByParam::parse(order_by, "created_at.desc");

        let query = match (order.column.as_str(), order.direction) {
            ("code", OrderDirection::Asc) => query.order((c_dsl::code.asc(), c_dsl::id.asc())),
            ("code", OrderDirection::Desc) => query.order((c_dsl::code.desc(), c_dsl::id.desc())),
            ("created_at", OrderDirection::Asc) => {
                query.order((c_dsl::created_at.asc(), c_dsl::id.asc()))
            }
            ("created_at", OrderDirection::Desc) => {
                query.order((c_dsl::created_at.desc(), c_dsl::id.desc()))
            }
            ("expires_at", OrderDirection::Asc) => {
                query.order((c_dsl::expires_at.asc().nulls_last(), c_dsl::id.asc()))
            }
            ("expires_at", OrderDirection::Desc) => {
                query.order((c_dsl::expires_at.desc().nulls_first(), c_dsl::id.desc()))
            }
            _ => query.order((c_dsl::created_at.desc(), c_dsl::id.desc())),
        };
        let query = query.paginate(pagination);

        crate::debug_sql(&query);

        query
            .load_and_count_pages(conn)
            .await
            .attach("Error while listing coupons")
            .into_db_result()
    }

    pub async fn delete(conn: &mut PgConn, tenant_id: TenantId, id: CouponId) -> DbResult<usize> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::delete(c_dsl::coupon)
            .filter(c_dsl::id.eq(id))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting coupon")
            .into_db_result()
    }

    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: &[CouponId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = c_dsl::coupon
            .filter(c_dsl::id.eq_any(ids))
            .filter(c_dsl::tenant_id.eq(tenant_id));

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching coupons: {e:?}"))
            .attach("Error while fetching coupons")
            .into_db_result()
    }

    /// Serializes funding the credit of these coupons' redemptions until the transaction ends.
    /// An advisory lock, not a row lock: settlement already holds key-share locks on the coupon
    /// through its applied coupons, which a row lock would have to wait for.
    pub async fn lock_redemption_funding(conn: &mut PgConn, ids: &[CouponId]) -> DbResult<()> {
        let mut ids = ids.to_vec();
        ids.sort_by_key(|id| id.as_uuid());
        ids.dedup();
        for id in ids {
            diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind::<diesel::sql_types::Text, _>(format!("coupon_redemption:{}", id.as_uuid()))
                .execute(conn)
                .await
                .attach("Error while locking a coupon's redemption funding")
                .into_db_result()?;
        }
        Ok(())
    }

    pub async fn list_by_ids_for_update(
        conn: &mut PgConn,
        ids: &[CouponId],
        tenant_id: &TenantId,
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        // Locked in id order, so transactions locking several coupons cannot deadlock.
        let query = c_dsl::coupon
            .filter(c_dsl::id.eq_any(ids))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .order(c_dsl::id)
            .for_update();

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while fetching coupons for update")
            .into_db_result()
    }

    pub async fn inc_redemption_count(
        conn: &mut PgConn,
        coupon_id: CouponId,
        delta: i32,
    ) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::update(c_dsl::coupon)
            .filter(c_dsl::id.eq(coupon_id))
            .set(c_dsl::redemption_count.eq(c_dsl::redemption_count + delta));

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while incrementing coupon redemption count")
            .into_db_result()
    }

    pub async fn customers_count(
        conn: &mut PgConn,
        coupons: &[CouponId],
    ) -> DbResult<HashMap<CouponId, i64>> {
        use crate::schema::applied_coupon::dsl as ac_dsl;

        let query = ac_dsl::applied_coupon
            .filter(ac_dsl::coupon_id.eq_any(coupons))
            .group_by(ac_dsl::coupon_id)
            .select((ac_dsl::coupon_id, diesel::dsl::count(ac_dsl::customer_id)));

        crate::debug_sql(&query);

        query
            .load::<(CouponId, i64)>(conn)
            .await
            .attach("Error while counting customers for coupons")
            .into_db_result()
            .map(|rows: Vec<(CouponId, i64)>| rows.into_iter().collect())
    }

    /// Dates a coupon from `created_at`, for a sandbox replay creating it as of a past moment.
    pub async fn set_created_at(
        conn: &mut PgConn,
        tenant_id: TenantId,
        coupon_id: CouponId,
        created_at: chrono::NaiveDateTime,
    ) -> DbResult<()> {
        use crate::schema::coupon::dsl as c_dsl;
        diesel::update(c_dsl::coupon)
            .filter(c_dsl::id.eq(coupon_id))
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .set(c_dsl::created_at.eq(created_at))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while dating a coupon")
            .into_db_result()
    }

    pub async fn update_last_redemption_at(
        conn: &mut PgConn,
        coupon_ids: &[CouponId],
        last_redemption_at: chrono::NaiveDateTime,
    ) -> DbResult<()> {
        use crate::schema::coupon::dsl as c_dsl;

        // Never moves back: a redemption recorded at a past moment keeps a later one.
        let query = diesel::update(c_dsl::coupon)
            .filter(c_dsl::id.eq_any(coupon_ids))
            .set(
                c_dsl::last_redemption_at.eq(diesel::dsl::sql::<
                    diesel::sql_types::Nullable<diesel::sql_types::Timestamp>,
                >("GREATEST(last_redemption_at, ")
                .bind::<diesel::sql_types::Timestamp, _>(last_redemption_at)
                .sql(")")),
            );

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while updating coupon last redemption at")
            .into_db_result()
    }

    /// The live coupon each of `codes` names (see [`CodeMatch::of`]), in order and without
    /// repeats; a code naming none, or several, names nothing here.
    pub async fn list_by_codes(
        conn: &mut PgConn,
        tenant_id: TenantId,
        codes: &[String],
    ) -> DbResult<Vec<CouponRow>> {
        let mut rows = Self::candidates_by_codes(conn, tenant_id, codes).await?;
        let mut ids: Vec<CouponId> = Vec::new();
        for code in codes {
            if let CodeMatch::Found(row) = CodeMatch::of(code, &rows)
                && !ids.contains(&row.id)
            {
                ids.push(row.id);
            }
        }
        let mut ordered = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(index) = rows.iter().position(|row| row.id == id) {
                ordered.push(rows.swap_remove(index));
            }
        }
        Ok(ordered)
    }

    /// The tenant's live coupons a code among `codes` may name: equal to it, or equal ignoring
    /// the case of ASCII letters.
    pub async fn candidates_by_codes(
        conn: &mut PgConn,
        tenant_id: TenantId,
        codes: &[String],
    ) -> DbResult<Vec<CouponRow>> {
        use crate::schema::coupon::dsl as c_dsl;

        diesel::define_sql_function! {
            fn translate(value: diesel::sql_types::Text, from: diesel::sql_types::Text, to: diesel::sql_types::Text) -> diesel::sql_types::Text;
        }
        diesel::define_sql_function! {
            fn btrim(value: diesel::sql_types::Text, characters: diesel::sql_types::Text) -> diesel::sql_types::Text;
        }

        let trimmed: Vec<String> = codes.iter().map(|code| code.trim().to_owned()).collect();
        let folded: Vec<String> = trimmed
            .iter()
            .map(|code| code.to_ascii_uppercase())
            .collect();
        // ASCII-only, as `CodeMatch::of` folds: `upper()` would also fold other letters.
        let query = c_dsl::coupon
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::archived_at.is_null())
            .filter(
                btrim(c_dsl::code, CODE_PADDING)
                    .eq_any(trimmed)
                    .or(translate(
                        btrim(c_dsl::code, CODE_PADDING),
                        "abcdefghijklmnopqrstuvwxyz",
                        "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
                    )
                    .eq_any(folded)),
            );

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing coupons by codes")
            .into_db_result()
    }

    /// Whether a live coupon of the tenant already uses `code` ignoring case; `credit_only` limits
    /// it to promotional-credit coupons.
    pub async fn code_taken_ignoring_case(
        conn: &mut PgConn,
        tenant_id: TenantId,
        code: &str,
        credit_only: bool,
    ) -> DbResult<bool> {
        use crate::schema::coupon::dsl as c_dsl;
        use diesel::PgJsonbExpressionMethods;

        diesel::define_sql_function! {
            fn upper(value: diesel::sql_types::Text) -> diesel::sql_types::Text;
        }
        diesel::define_sql_function! {
            fn btrim(value: diesel::sql_types::Text, characters: diesel::sql_types::Text) -> diesel::sql_types::Text;
        }

        let mut query = c_dsl::coupon
            .filter(c_dsl::tenant_id.eq(tenant_id))
            .filter(c_dsl::archived_at.is_null())
            .filter(upper(btrim(c_dsl::code, CODE_PADDING)).eq(code.trim().to_uppercase()))
            .into_boxed();
        if credit_only {
            query = query.filter(c_dsl::discount.has_key(CREDIT_DISCOUNT_KEY));
        }

        diesel::select(diesel::dsl::exists(query.select(c_dsl::id)))
            .get_result(conn)
            .await
            .attach("Error while checking a coupon code")
            .into_db_result()
    }
}

impl CouponRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::update(c_dsl::coupon)
            .filter(c_dsl::id.eq(self.id))
            .filter(c_dsl::tenant_id.eq(self.tenant_id))
            .set(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating coupon")
            .into_db_result()
    }
}

impl CouponStatusRowPatch {
    pub async fn patch(&self, conn: &mut PgConn) -> DbResult<CouponRow> {
        use crate::schema::coupon::dsl as c_dsl;

        let query = diesel::update(c_dsl::coupon)
            .filter(c_dsl::id.eq(self.id))
            .filter(c_dsl::tenant_id.eq(self.tenant_id))
            .set(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating coupon status")
            .into_db_result()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn code_padding_is_what_trim_removes() {
        let whitespace: String = (0..=u32::from(char::MAX))
            .filter_map(char::from_u32)
            .filter(|c| c.is_whitespace())
            .collect();
        assert_eq!(super::CODE_PADDING, whitespace);
    }
}
