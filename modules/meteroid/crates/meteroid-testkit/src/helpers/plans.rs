use common_domain::ids::PlanVersionId;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::schema::plan_version::dsl as pv;
use meteroid_store::Store;

/// Runs `write` with `version` reopened as a draft, then published again: fixtures that build a
/// published catalog by hand, as the store only lets a draft's components change.
pub async fn as_draft<T>(
    store: &Store,
    version: PlanVersionId,
    write: impl std::future::Future<Output = T>,
) -> T {
    let mut conn = store.get_conn().await.expect("connection");
    let was_draft: bool = pv::plan_version
        .filter(pv::id.eq(version))
        .select(pv::is_draft_version)
        .first(&mut conn)
        .await
        .expect("the plan version");
    drop(conn);
    let set = |draft: bool| async move {
        diesel::update(pv::plan_version.filter(pv::id.eq(version)))
            .set(pv::is_draft_version.eq(draft))
            .execute(&mut store.get_conn().await.expect("connection"))
            .await
            .expect("the plan version's draft flag");
    };
    if !was_draft {
        set(true).await;
    }
    let written = write.await;
    if !was_draft {
        set(false).await;
    }
    written
}
