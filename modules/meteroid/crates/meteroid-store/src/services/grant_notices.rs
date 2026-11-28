//! `grant.ended` for a grant whose window is over on its own (R-178): once per grant of each
//! application, its items ended at the window's end. Grants ended with their subscription, or
//! withdrawn, are noticed where that happens.
use chrono::{DateTime, Utc};
use diesel_models::offers::{OfferApplicationItemRow, OfferApplicationRow};
use error_stack::Report;
use itertools::Itertools;
use scoped_futures::ScopedFutureExt;

use crate::domain::usage_alert_events::GrantEndReason;
use crate::errors::StoreError;
use crate::{Store, StoreResult};

const BATCH: i64 = 200;

/// Publishes `grant.ended` for the windows over at `now`; how many grants.
pub async fn notify_grant_windows_ended(store: &Store, now: DateTime<Utc>) -> StoreResult<usize> {
    let mut total = 0;
    loop {
        let claimed = store
            .transaction(|conn| {
                async move {
                    let ended =
                        OfferApplicationItemRow::claim_windows_ended(conn, now.naive_utc(), BATCH)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    let ids = ended
                        .iter()
                        .map(|i| i.application_id)
                        .unique()
                        .collect_vec();
                    let applications = OfferApplicationRow::list_by_ids(conn, &ids)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                    let grants = ended
                        .iter()
                        .map(|i| (i.application_id, i.grant_id))
                        .unique();
                    let count = grants.count();
                    let by_tenant = ended.into_iter().into_group_map_by(|i| i.tenant_id);
                    for (tenant_id, items) in by_tenant {
                        crate::services::offers::apply::publish_ended_tx(
                            conn,
                            &applications,
                            &items,
                            GrantEndReason::WindowEnded,
                            tenant_id,
                        )
                        .await?;
                    }
                    Ok(count)
                }
                .scope_boxed()
            })
            .await?;
        total += claimed;
        if claimed < BATCH as usize {
            return Ok(total);
        }
    }
}
