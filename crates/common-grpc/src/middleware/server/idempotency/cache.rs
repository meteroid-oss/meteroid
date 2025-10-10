use std::any::type_name;
use std::future::Future;

use tonic::metadata::MetadataMap;
use tonic::{Request, Response, Status};

use crate::middleware::common::idempotency::{
    IDEMPOTENCY_CACHE_RESPONSE_HEADER, IDEMPOTENCY_KEY_HEADER,
};
use crate::middleware::server::auth::RequestExt;

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
struct CacheKey {
    path: String,
    idempotency_key: String,
    maybe_actor: Option<uuid::Uuid>,
}

#[derive(Clone, Debug)]
enum CacheValue {
    InProgress,
    Cached(Result<(MetadataMap, Vec<u8>), Status>),
}

#[derive(Clone, Debug)]
enum ActionDirective {
    Ignore,
    LoadToCache(CacheKey),
    GetFromCache(Result<(MetadataMap, Vec<u8>), Status>),
}

const VALUE_MIN_LEN: usize = 8;

const VALUE_MAX_LEN: usize = 64;

static GRPC_IDEMPOTENCY_CACHE: std::sync::LazyLock<moka::sync::Cache<CacheKey, CacheValue>> =
    std::sync::LazyLock::new(|| {
        let config = common_config::idempotency::IdempotencyConfig::get();
        moka::sync::Cache::builder()
            .max_capacity(config.size)
            .time_to_live(config.ttl.into())
            .build()
    });

pub async fn idempotency_cache<F, Fut, Req, Res>(
    request: Request<Req>,
    thunk: F,
) -> Result<Response<Res>, Status>
where
    F: FnOnce(Request<Req>) -> Fut,
    Fut: Future<Output = Result<Response<Res>, Status>>,
    Req: Clone + Default + ::prost::Message,
    Res: Clone + Default + ::prost::Message,
{
    let cache = GRPC_IDEMPOTENCY_CACHE.clone();

    let config = common_config::idempotency::IdempotencyConfig::get();

    let parsed_idempotency_key = match request.metadata().get(IDEMPOTENCY_KEY_HEADER) {
        None => {
            if config.required {
                Err(Status::failed_precondition("Idempotency header not found"))
            } else {
                Ok(None)
            }
        }
        Some(header_value) => header_value
            .to_str()
            .map_err(|_| Status::invalid_argument("Can't process idempotency header value"))
            .and_then(|v| {
                if v.len() >= VALUE_MIN_LEN && v.len() <= VALUE_MAX_LEN {
                    Ok(Some(v))
                } else {
                    Err(Status::invalid_argument("Invalid idempotency value length"))
                }
            }),
    };

    let error_or_action_directive = parsed_idempotency_key.and_then(|maybe_idempotency_key| {
        match maybe_idempotency_key {
            None => {
                // do nothing because idempotency header is not required
                Ok(ActionDirective::Ignore)
            }
            Some(idempotency_key) => {
                let path = type_name::<Req>();
                let maybe_actor = request.actor().ok();
                let cache_key = CacheKey {
                    path: path.to_string(),
                    idempotency_key: idempotency_key.to_string(),
                    maybe_actor,
                };
                // todo this is not thread safe, we will move it behind a trait and make sure it is thread safe there
                match cache.get(&cache_key) {
                    None => {
                        // 1st call
                        cache
                            .clone()
                            .insert(cache_key.clone(), CacheValue::InProgress);

                        Ok(ActionDirective::LoadToCache(cache_key))
                    }
                    Some(CacheValue::InProgress) => {
                        Err(Status::already_exists("Request already in progress"))
                    }
                    Some(CacheValue::Cached(result)) => Ok(ActionDirective::GetFromCache(result)),
                }
            }
        }
    })?;

    if let ActionDirective::GetFromCache(result) = error_or_action_directive {
        return match result {
            Ok((metadata, message)) => {
                let res = Res::decode(message.as_slice()).unwrap();
                let response = Response::from_parts(metadata, res, tonic::Extensions::default());
                Ok(response)
            }
            Err(status) => Err(status),
        };
    }

    let result = thunk(request).await;

    if let ActionDirective::LoadToCache(key) = error_or_action_directive {
        match result {
            Ok(response) => {
                let (mut metadata, message, extension) = response.into_parts();

                metadata.insert(IDEMPOTENCY_CACHE_RESPONSE_HEADER, "cache".parse().unwrap());

                let cache_value = Ok((metadata.clone(), message.encode_to_vec()));
                cache.insert(key, CacheValue::Cached(cache_value));

                metadata.insert(
                    IDEMPOTENCY_CACHE_RESPONSE_HEADER,
                    "original".parse().unwrap(),
                );

                Ok(Response::from_parts(metadata.clone(), message, extension))
            }
            Err(mut status) => {
                status
                    .metadata_mut()
                    .insert(IDEMPOTENCY_CACHE_RESPONSE_HEADER, "cache".parse().unwrap());

                let cache_value = Err(status.clone());
                cache.insert(key, CacheValue::Cached(cache_value));

                status.metadata_mut().insert(
                    IDEMPOTENCY_CACHE_RESPONSE_HEADER,
                    "original".parse().unwrap(),
                );

                Err(status)
            }
        }
    } else {
        result
    }
}
