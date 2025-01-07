use cached::once_cell::sync::Lazy;

use quick_cache::sync::Cache;
use std::sync::Arc;

// type IdentifierCache = Lazy<RwLock<SizedCache<(String, String), String>>>;
// pub static CUSTOMER_ID_CACHE: IdentifierCache = Lazy::new(|| RwLock::new(SizedCache::with_size(10000)));
type TenantAliasTuple = (String, String);
type IdentifierCache = Lazy<Arc<Cache<TenantAliasTuple, String>>>;
pub static CUSTOMER_ID_CACHE: IdentifierCache = Lazy::new(|| Arc::new(Cache::new(10000)));

// TODO add an optional redis on top
