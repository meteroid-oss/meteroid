use cached::once_cell::sync::Lazy;
use common_domain::ids::{CustomerId, TenantId};
use quick_cache::sync::Cache;
use std::sync::Arc;

type TenantAliasTuple = (TenantId, String);
type IdentifierCache = Lazy<Arc<Cache<TenantAliasTuple, CustomerId>>>;
pub static CUSTOMER_ID_CACHE: IdentifierCache = Lazy::new(|| Arc::new(Cache::new(10000)));

// TODO add an optional redis on top
