use deadpool_postgres::{Object, Pool};
use tonic::Status;

#[tracing::instrument(skip(pool))]
pub async fn get_connection(pool: &Pool) -> Result<Object, Status> {
    match pool.get().await {
        Ok(client) => Ok(client),
        Err(e) => {
            log::error!("Unable to get database connection : {e}");
            Err(Status::unavailable("Unable to get database connection"))
        }
    }
}
