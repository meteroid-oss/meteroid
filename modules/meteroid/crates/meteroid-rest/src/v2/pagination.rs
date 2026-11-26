//! Cursor pagination (ADR-0013 §3): `?limit=&after=` answering `{data, next_cursor, has_more}`.
//!
//! A cursor is opaque to callers: the base64url of the last id of the page. The catalog lists
//! are bounded per tenant and paged here by keyset over the id (UUIDv7, so in creation order). An
//! unbounded collection (subscriptions, invoices) pages in SQL instead, `id > cursor ORDER BY id
//! LIMIT limit + 1`, with the same cursor ([`Cursor`]). A list whose order is not its ids' (credit
//! grants by creation, a credit ledger by effective date) pages by keyset over the columns it is
//! ordered by, its cursor carrying the last item's key ([`KeyCursor`]), as opaque.
use crate::v2::problem::Problem;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

pub const DEFAULT_LIMIT: u32 = 20;
pub const MAX_LIMIT: u32 = 100;

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    /// How many items, 1 to 100; 20 by default.
    pub limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    pub after: Option<String>,
}

/// Where a page starts: after the item of this id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor(pub Uuid);

impl Cursor {
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0.to_string())
    }

    pub fn decode(cursor: &str) -> Result<Self, Problem> {
        let refused = || Problem::field("after", "invalid", "Not a cursor of this list");
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor.trim())
            .map_err(|_| refused())?;
        let text = String::from_utf8(bytes).map_err(|_| refused())?;
        Uuid::parse_str(&text).map(Cursor).map_err(|_| refused())
    }
}

/// Where a page of a list ordered by several columns starts: after the item of this key, each
/// part the text of one column (a moment in microseconds, a number, an id). Its first part names
/// the list, so a cursor of another list is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCursor {
    pub parts: Vec<String>,
}

impl KeyCursor {
    const SEPARATOR: char = '|';

    /// The cursor of `list` after the item whose key is `parts`; no part holds the separator.
    pub fn new(list: &str, parts: Vec<String>) -> Self {
        let mut all = Vec::with_capacity(parts.len() + 1);
        all.push(list.to_owned());
        all.extend(parts);
        Self { parts: all }
    }

    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.parts.join(&Self::SEPARATOR.to_string()))
    }

    /// The key parts of a cursor of `list` with `arity` of them.
    pub fn decode(cursor: &str, list: &str, arity: usize) -> Result<Vec<String>, Problem> {
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor.trim())
            .map_err(|_| refused_cursor())?;
        let text = String::from_utf8(bytes).map_err(|_| refused_cursor())?;
        let mut parts: Vec<String> = text.split(Self::SEPARATOR).map(str::to_owned).collect();
        if parts.len() != arity + 1 || parts[0] != list {
            return Err(refused_cursor());
        }
        Ok(parts.split_off(1))
    }

    /// A moment as a key part: microseconds since the epoch, Postgres' precision.
    pub fn moment(at: DateTime<Utc>) -> String {
        at.timestamp_micros().to_string()
    }

    pub fn parse_moment(part: &str) -> Result<DateTime<Utc>, Problem> {
        part.parse::<i64>()
            .ok()
            .and_then(DateTime::from_timestamp_micros)
            .ok_or_else(refused_cursor)
    }

    pub fn parse_id(part: &str) -> Result<Uuid, Problem> {
        Uuid::parse_str(part).map_err(|_| refused_cursor())
    }
}

/// What a cursor that is not one of the list's answers.
pub fn refused_cursor() -> Problem {
    Problem::field("after", "invalid", "Not a cursor of this list")
}

impl PageQuery {
    /// The `after` of the query, none when left out or blank.
    pub fn after(&self) -> Option<&str> {
        self.after
            .as_deref()
            .filter(|after| !after.trim().is_empty())
    }

    pub fn limit(&self) -> Result<usize, Problem> {
        match self.limit {
            None => Ok(DEFAULT_LIMIT as usize),
            Some(limit) if (1..=MAX_LIMIT).contains(&limit) => Ok(limit as usize),
            Some(_) => Err(Problem::field(
                "limit",
                "invalid",
                format!("limit is 1 to {MAX_LIMIT}"),
            )),
        }
    }

    pub fn cursor(&self) -> Result<Option<Cursor>, Problem> {
        self.after
            .as_deref()
            .filter(|after| !after.trim().is_empty())
            .map(Cursor::decode)
            .transpose()
    }
}

/// A page of a list, its next cursor when more follow.
#[derive(Debug, Clone)]
pub struct Paged<T> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl<T> Paged<T> {
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Paged<U> {
        Paged {
            data: self.data.into_iter().map(f).collect(),
            next_cursor: self.next_cursor,
            has_more: self.has_more,
        }
    }
}

/// The page of `items` `query` asks for, the items ordered by `key` (their id).
pub fn page<T>(
    mut items: Vec<T>,
    key: impl Fn(&T) -> Uuid,
    query: &PageQuery,
) -> Result<Paged<T>, Problem> {
    let limit = query.limit()?;
    let cursor = query.cursor()?;
    items.sort_by_key(|item| key(item));
    let mut data: Vec<T> = items
        .into_iter()
        .filter(|item| cursor.is_none_or(|Cursor(after)| key(item) > after))
        .take(limit + 1)
        .collect();
    let has_more = data.len() > limit;
    data.truncate(limit);
    let next_cursor = if has_more {
        data.last().map(|item| Cursor(key(item)).encode())
    } else {
        None
    };
    Ok(Paged {
        data,
        next_cursor,
        has_more,
    })
}

/// The page of rows a store read in SQL by keyset (`id > cursor ORDER BY id LIMIT limit + 1`):
/// the row past `limit` only tells that more follow.
pub fn keyset<T>(mut rows: Vec<T>, limit: usize, key: impl Fn(&T) -> Uuid) -> Paged<T> {
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    let next_cursor = if has_more {
        rows.last().map(|row| Cursor(key(row)).encode())
    } else {
        None
    };
    Paged {
        data: rows,
        next_cursor,
        has_more,
    }
}

/// A list response type: `{data, next_cursor, has_more}` of `$item`.
#[macro_export]
macro_rules! v2_list {
    ($name:ident, $item:ty) => {
        #[derive(serde::Serialize, utoipa::ToSchema, Debug, Clone)]
        #[schema(description = concat!(
            "A page of `", stringify!($item), "` items, by cursor: pass `next_cursor` as `after` \
             for the next."
        ))]
        pub struct $name {
            /// The items of the page, in the list's order.
            pub data: Vec<$item>,
            /// Pass it as `after` for the next page; none on the last.
            pub next_cursor: Option<String>,
            /// Whether another page follows: `next_cursor` is set when it does.
            pub has_more: bool,
        }

        impl From<$crate::v2::pagination::Paged<$item>> for $name {
            fn from(page: $crate::v2::pagination::Paged<$item>) -> Self {
                $name {
                    data: page.data,
                    next_cursor: page.next_cursor,
                    has_more: page.has_more,
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_follow_each_other_by_cursor() {
        let ids: Vec<Uuid> = (1..=5).map(Uuid::from_u128).collect();
        let query = PageQuery {
            limit: Some(2),
            after: None,
        };
        let first = page(ids.clone(), |id| *id, &query).unwrap();
        assert_eq!(first.data, ids[..2]);
        assert!(first.has_more);
        let second = page(
            ids.clone(),
            |id| *id,
            &PageQuery {
                limit: Some(2),
                after: first.next_cursor,
            },
        )
        .unwrap();
        assert_eq!(second.data, ids[2..4]);
        let last = page(
            ids.clone(),
            |id| *id,
            &PageQuery {
                limit: Some(2),
                after: second.next_cursor,
            },
        )
        .unwrap();
        assert_eq!(last.data, ids[4..]);
        assert!(!last.has_more);
        assert_eq!(last.next_cursor, None);
    }

    #[test]
    fn a_limit_out_of_range_is_refused() {
        let query = PageQuery {
            limit: Some(0),
            after: None,
        };
        assert!(page(Vec::<Uuid>::new(), |id| *id, &query).is_err());
        assert!(Cursor::decode("not a cursor").is_err());
    }

    #[test]
    fn a_key_cursor_reads_back_its_parts_and_names_its_list() {
        let at = DateTime::from_timestamp_micros(1_764_000_000_123_456).unwrap();
        let id = Uuid::from_u128(7);
        let cursor = KeyCursor::new(
            "ledger",
            vec![KeyCursor::moment(at), "1".to_owned(), id.to_string()],
        )
        .encode();
        let parts = KeyCursor::decode(&cursor, "ledger", 3).unwrap();
        assert_eq!(KeyCursor::parse_moment(&parts[0]).unwrap(), at);
        assert_eq!(parts[1], "1");
        assert_eq!(KeyCursor::parse_id(&parts[2]).unwrap(), id);
        assert!(KeyCursor::decode(&cursor, "grants", 3).is_err());
        assert!(KeyCursor::decode(&cursor, "ledger", 2).is_err());
        assert!(KeyCursor::decode(&Cursor(id).encode(), "ledger", 3).is_err());
    }
}
