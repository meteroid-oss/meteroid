//! The document corpus: a plain-text rendering of every invoice, credit note and credit grant a
//! test left behind, compared with `<name>.txt` in the directory `METEROID_BILLING_CORPUS` names.
//! A change to billing output shows up as a diff of that file in review, the way an invoice reads.
//!
//! Without that directory nothing is compared. `CORPUS_UPDATE=1` rewrites the files. Outside CI a
//! missing file is written; in CI it fails.
use diesel::sql_types::Text;
use diesel_async::RunQueryDsl;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(diesel::QueryableByName)]
struct Row {
    #[diesel(sql_type = Text)]
    line: String,
}

const INVOICES: &str = "
WITH inv AS (
    SELECT i.*, row_number() OVER (ORDER BY i.invoice_date, i.created_at, i.id) AS n
    FROM invoice i WHERE i.tenant_id = $1),
sub AS (
    SELECT s.id, row_number() OVER (ORDER BY s.created_at, s.id) AS n
    FROM subscription s WHERE s.tenant_id = $1)
SELECT line FROM (
    SELECT i.n, 0 AS part, 0 AS ord, format(
        'invoice #%s %s %s %s %s, %s, sub #%s, dated %s, period from %s, %s %s',
        i.n, i.invoice_type, i.status, i.payment_status, i.currency, c.name,
        coalesce(s.n::text, '-'), i.invoice_date, i.billing_period_start,
        CASE WHEN i.parent_invoice_id IS NULL THEN '' ELSE 'corrects #' || p.n END,
        CASE WHEN i.consolidated_into_invoice_id IS NULL THEN '' ELSE 'merged into #' || m.n END
    ) AS line
    FROM inv i
    JOIN customer c ON c.id = i.customer_id
    LEFT JOIN sub s ON s.id = i.subscription_id
    LEFT JOIN inv p ON p.id = i.parent_invoice_id
    LEFT JOIN inv m ON m.id = i.consolidated_into_invoice_id
    UNION ALL
    SELECT i.n, 1, l.ord, format(
        '  %s [%s] %s..%s qty %s x %s = %s, taxable %s, tax %s, total %s, promotional %s',
        l.v->>'name', coalesce(l.v->>'kind', 'Regular'), l.v->>'start_date', l.v->>'end_date',
        coalesce(l.v->>'quantity', '-'), coalesce(l.v->>'unit_price', '-'),
        coalesce(l.v->>'amount_subtotal', l.v->>'subtotal'), l.v->>'taxable_amount',
        l.v->>'tax_amount', coalesce(l.v->>'amount_total', l.v->>'total'),
        coalesce(l.v->>'promotional_credit', '0'))
    FROM inv i, jsonb_array_elements(i.line_items) WITH ORDINALITY AS l(v, ord)
    UNION ALL
    SELECT i.n, 2, b.ord, format('  tax %s %s on %s = %s', b.v->>'name', b.v->>'tax_rate',
                                 b.v->>'taxable_amount', coalesce(b.v->>'tax_amount', '0'))
    FROM inv i, jsonb_array_elements(i.tax_breakdown) WITH ORDINALITY AS b(v, ord)
    UNION ALL
    SELECT i.n, 3, 0, format(
        '  subtotal %s, discount %s, promotional %s, tax %s, total %s, applied %s, due %s',
        i.subtotal, i.discount, i.promotional_credits, i.tax_amount, i.total,
        i.applied_credits, i.amount_due)
    FROM inv i
) x ORDER BY n, part, ord";

const CREDIT_NOTES: &str = "
WITH inv AS (
    SELECT i.id, row_number() OVER (ORDER BY i.invoice_date, i.created_at, i.id) AS n
    FROM invoice i WHERE i.tenant_id = $1),
note AS (
    SELECT n.*, row_number() OVER (ORDER BY n.created_at, n.id) AS k
    FROM credit_note n WHERE n.tenant_id = $1)
SELECT line FROM (
    SELECT n.k, 0 AS part, 0 AS ord, format(
        'credit note #%s %s %s on invoice #%s%s: subtotal %s, tax %s, total %s, refunded %s, credited %s, debt cancelled %s',
        n.k, n.status, n.credit_type, i.n, CASE WHEN n.voids_invoice THEN ' (void)' ELSE '' END,
        n.subtotal, n.tax_amount, n.total, n.refunded_amount_cents, n.credited_amount_cents,
        n.debt_cancelled_amount_cents) AS line
    FROM note n JOIN inv i ON i.id = n.invoice_id
    UNION ALL
    SELECT n.k, 1, l.ord, format(
        '  %s [%s] %s..%s qty %s x %s = %s, taxable %s, tax %s, total %s',
        l.v->>'name', coalesce(l.v->>'kind', 'Regular'), l.v->>'start_date', l.v->>'end_date',
        coalesce(l.v->>'quantity', '-'), coalesce(l.v->>'unit_price', '-'),
        coalesce(l.v->>'amount_subtotal', l.v->>'subtotal'), l.v->>'taxable_amount',
        l.v->>'tax_amount', coalesce(l.v->>'amount_total', l.v->>'total'))
    FROM note n, jsonb_array_elements(n.line_items) WITH ORDINALITY AS l(v, ord)
) x ORDER BY k, part, ord";

const GRANTS: &str = "
WITH inv AS (
    SELECT i.id, row_number() OVER (ORDER BY i.invoice_date, i.created_at, i.id) AS n
    FROM invoice i WHERE i.tenant_id = $1),
sub AS (
    SELECT s.id, row_number() OVER (ORDER BY s.created_at, s.id) AS n
    FROM subscription s WHERE s.tenant_id = $1),
g AS (
    SELECT g.*, row_number() OVER (ORDER BY g.created_at, g.id) AS k
    FROM customer_credit_grant g WHERE g.tenant_id = $1)
SELECT line FROM (
    SELECT g.k, 0 AS part, '' AS kind, format(
        'grant #%s %s %s %s from %s: %s, %s left, cost %s %s, effective %s, expires %s, ends %s%s%s',
        g.k, g.classification, g.application, g.source_kind, c.name, g.amount, g.remaining,
        coalesce(g.cost_minor::text, '-'), coalesce(g.cost_currency, ''),
        g.effective_at, coalesce(g.expires_at::text, '-'), coalesce(g.ends_at::text, '-'),
        coalesce(', invoice #' || i.n, ''), coalesce(', sub #' || s.n, '')) AS line
    FROM g
    JOIN customer c ON c.id = g.customer_id
    LEFT JOIN inv i ON i.id = g.invoice_id
    LEFT JOIN sub s ON s.id = g.subscription_id
    UNION ALL
    SELECT g.k, 1, e.kind, format('  %s %s', e.kind, sum(e.amount))
    FROM g JOIN customer_credit_entry e ON e.grant_id = g.id
    GROUP BY g.k, e.kind
) x ORDER BY k, part, kind";

/// The tenant's documents and grants, rendered with ids replaced by creation order.
pub async fn render(conn: &mut meteroid_store::store::PgConn, tenant: Uuid) -> String {
    let mut out = String::new();
    for (title, sql) in [
        ("Invoices", INVOICES),
        ("Credit notes", CREDIT_NOTES),
        ("Credit grants", GRANTS),
    ] {
        let rows: Vec<Row> = diesel::sql_query(sql)
            .bind::<diesel::sql_types::Uuid, _>(tenant)
            .load(conn)
            .await
            .unwrap_or_else(|error| panic!("corpus query `{title}` failed: {error:?}"));
        out.push_str(&format!("## {title}\n"));
        for row in rows {
            out.push_str(row.line.trim_end());
            out.push('\n');
        }
        out.push('\n');
    }
    relative_to(&out, chrono::Utc::now().date_naive())
}

/// Writes dates near `today` as `today+N`, without their time of day: a test that cannot set the
/// clock stamps documents with the day it runs, and its corpus must not change the next day.
/// Business dates of tests (2024, 2025) are kept as they are.
fn relative_to(text: &str, today: chrono::NaiveDate) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let date = text
            .get(i..i + 10)
            .filter(|_| i == 0 || !bytes[i - 1].is_ascii_alphanumeric())
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
        let offset = date.map(|d| (d - today).num_days());
        match offset {
            Some(days) if (-2..=730).contains(&days) => {
                out.push_str(&format!("today{days:+}"));
                i += 10;
                let rest = &text[i..];
                let time_len = rest
                    .strip_prefix(' ')
                    .map(|r| {
                        r.find(|c: char| !(c.is_ascii_digit() || ":.+".contains(c)))
                            .unwrap_or(r.len())
                    })
                    .filter(|len| *len >= 8)
                    .map_or(0, |len| len + 1);
                i += time_len;
            }
            _ => {
                let ch = text[i..].chars().next().expect("in bounds");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

/// Compares the rendering of `tenant` in the database at `url` with `<name>.txt`.
/// Runs on a thread of its own, so it can be called while the test's runtime tears down.
pub fn assert_matches(url: &str, tenant: Uuid, name: &str) {
    let Some(dir) = dir() else { return };
    let url = url.to_owned();
    let rendered = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("corpus runtime")
            .block_on(async move {
                let pool = meteroid_store::store::diesel_make_pg_pool(
                    meteroid_store::store::PgConfig::new(url),
                )
                .expect("corpus pool");
                let mut conn = pool.get().await.expect("corpus connection");
                render(&mut conn, tenant).await
            })
    })
    .join()
    .expect("the corpus rendering ran");
    compare(&dir, name, &rendered);
}

/// Compares the rendering of `tenant` with `<name>.txt`, from inside the test.
pub async fn assert_tenant(conn: &mut meteroid_store::store::PgConn, tenant: Uuid, name: &str) {
    let Some(dir) = dir() else { return };
    let rendered = render(conn, tenant).await;
    compare(&dir, name, &rendered);
}

fn dir() -> Option<PathBuf> {
    std::env::var_os("METEROID_BILLING_CORPUS")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
}

fn compare(dir: &std::path::Path, name: &str, rendered: &str) {
    let path = dir.join(format!("{name}.txt"));
    let update = std::env::var_os("CORPUS_UPDATE").is_some();
    let expected = std::fs::read_to_string(&path).ok();
    if update || (expected.is_none() && std::env::var_os("CI").is_none()) {
        std::fs::write(&path, rendered).expect("write the corpus file");
        return;
    }
    let Some(expected) = expected else {
        panic!(
            "no corpus file {}: run the test with CORPUS_UPDATE=1",
            path.display()
        );
    };
    if expected != rendered {
        let new = path.with_extension("txt.new");
        let _ = std::fs::write(&new, rendered);
        let diff: Vec<String> = expected
            .lines()
            .zip(rendered.lines())
            .enumerate()
            .filter(|(_, (e, r))| e != r)
            .take(10)
            .map(|(i, (e, r))| format!("line {}:\n  - {e}\n  + {r}", i + 1))
            .collect();
        panic!(
            "billing output changed for {name} ({} vs {} lines); new rendering in {}\n{}\n\
             If intended, accept it with CORPUS_UPDATE=1 and record why with the rules.",
            expected.lines().count(),
            rendered.lines().count(),
            new.display(),
            diff.join("\n")
        );
    }
}

/// The corpus name of the running test: its path without the leading module, e.g.
/// `scenarios::s15_minimum_commitment::s15_x` gives `s15_minimum_commitment__s15_x`.
pub fn current_test_name() -> Option<String> {
    let thread = std::thread::current();
    // A test runs on a thread named after its path; runtime workers ("tokio-runtime-worker") are
    // not tests.
    let name = thread.name().filter(|name| name.contains("::"))?;
    let path: Vec<&str> = name.split("::").collect();
    let start = path
        .iter()
        .position(|part| *part == "scenarios")
        .map_or(0, |i| i + 1);
    let parts = &path[start..];
    (!parts.is_empty()).then(|| parts.join("__"))
}

/// `name` the first time it is claimed in this process, then `name__2`, `name__3`…: a test that
/// runs several cases, each with its own tenant, gets one file per case.
pub fn claim(name: &str) -> String {
    static CLAIMED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut claimed = CLAIMED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let taken = claimed.iter().filter(|c| c.as_str() == name).count();
    claimed.push(name.to_owned());
    match taken {
        0 => name.to_owned(),
        n => format!("{name}__{}", n + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_near_today_are_relative() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
        assert_eq!(
            relative_to(
                "dated 2026-11-04, ..2026-11-05 qty, effective 2026-11-04 02:45:45.371044+00, \
                 expires 2026-11-29 02:45:45+00, ends -, kept 2025-09-01 00:00:00+00",
                today
            ),
            "dated today+0, ..today+1 qty, effective today+0, expires today+25, ends -, \
             kept 2025-09-01 00:00:00+00"
        );
    }
}
