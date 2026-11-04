//! Waiting for ingested usage to become queryable, bounded by wall-clock time whatever the
//! metering backend does.

use std::future::Future;
use std::time::Duration;

use futures::future::join_all;
use rust_decimal::Decimal;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// A default sandbox's sample usage is a demo extra: a short wait, then seeding goes on.
pub(super) const DEMO_USAGE_WAIT: Duration = Duration::from_secs(5);
/// Credit history is billed from its usage: waited for longer, and seeding stops without it.
pub(super) const CREDIT_USAGE_WAIT: Duration = Duration::from_secs(90);

/// Reads every pending total concurrently, again and again, until all match or `within` runs out.
/// Returns what is still short.
pub(super) async fn await_totals<'a, T, F, Fut, E>(
    items: &'a [T],
    within: Duration,
    expected: impl Fn(&T) -> Decimal,
    label: impl Fn(&T) -> String,
    fetch: F,
) -> Result<(), Vec<String>>
where
    F: Fn(&'a T) -> Fut,
    Fut: Future<Output = Result<Decimal, E>>,
    E: std::fmt::Debug,
{
    let mut short: Vec<String> = items
        .iter()
        .map(|item| format!("{}: not read", label(item)))
        .collect();
    let mut pending: Vec<&'a T> = items.iter().collect();
    let polled = tokio::time::timeout(within, async {
        loop {
            let reads = join_all(pending.iter().map(|item| fetch(item))).await;
            let mut still = Vec::new();
            short.clear();
            for (item, read) in pending.iter().zip(reads) {
                let want = expected(item);
                match read {
                    Ok(total) if (total - want).abs() <= tolerance(want) => {}
                    Ok(total) => {
                        short.push(format!("{}: {total} of {want}", label(item)));
                        still.push(*item);
                    }
                    Err(e) => {
                        short.push(format!("{}: {e:?}", label(item)));
                        still.push(*item);
                    }
                }
            }
            pending = still;
            if pending.is_empty() {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await;
    match polled {
        Ok(()) => Ok(()),
        Err(_) => Err(short),
    }
}

/// The meter sums in floating point; a fraction of a unit is not missing usage.
fn tolerance(total: Decimal) -> Decimal {
    (total.abs() * Decimal::new(1, 9)).max(Decimal::new(1, 2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    type Item = (&'static str, Decimal);

    fn want(item: &Item) -> Decimal {
        item.1
    }

    fn label(item: &Item) -> String {
        item.0.to_string()
    }

    #[tokio::test(start_paused = true)]
    async fn a_hanging_meter_is_given_up_on_at_the_deadline() {
        let items: Vec<Item> = vec![("a", dec!(1)), ("b", dec!(2))];
        let started = tokio::time::Instant::now();
        let result = await_totals(&items, DEMO_USAGE_WAIT, want, label, |_| {
            std::future::pending::<Result<Decimal, ()>>()
        })
        .await;
        assert_eq!(result.unwrap_err(), vec!["a: not read", "b: not read"]);
        let elapsed = started.elapsed();
        assert!(elapsed >= DEMO_USAGE_WAIT && elapsed < DEMO_USAGE_WAIT + Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn totals_are_read_concurrently_and_retried_until_they_match() {
        // Three reads of 10s each would miss a 15s deadline one after the other.
        let items: Vec<Item> = vec![("a", dec!(1)), ("b", dec!(2)), ("c", dec!(3))];
        let slow = |item: &Item| {
            let total = item.1;
            async move {
                tokio::time::sleep(Duration::from_secs(10)).await;
                Ok::<_, ()>(total)
            }
        };
        assert!(
            await_totals(&items, Duration::from_secs(15), want, label, slow)
                .await
                .is_ok()
        );

        let short = |item: &Item| {
            let total = item.1 - Decimal::ONE;
            async move { Ok::<_, ()>(total) }
        };
        let missing = await_totals(&items, Duration::from_secs(2), want, label, short)
            .await
            .unwrap_err();
        assert_eq!(missing, vec!["a: 0 of 1", "b: 1 of 2", "c: 2 of 3"]);
    }
}
