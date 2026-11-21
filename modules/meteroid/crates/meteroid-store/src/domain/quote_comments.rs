use chrono::NaiveDateTime;
use common_domain::ids::{QuoteCommentId, QuoteId, UserId};
use diesel_models::quote_comments::QuoteCommentRow;

pub const MAX_COMMENT_LENGTH: usize = 10_000;
pub const MAX_ANCHOR_LENGTH: usize = 200;

#[derive(Debug, Clone)]
pub struct QuoteComment {
    pub id: QuoteCommentId,
    pub quote_id: QuoteId,
    pub revision: i32,
    pub author_user_id: Option<UserId>,
    pub author_email: Option<String>,
    pub author_name: String,
    pub body: String,
    pub shared: bool,
    pub is_change_request: bool,
    pub resolved_at: Option<NaiveDateTime>,
    pub resolved_by: Option<UserId>,
    pub created_at: NaiveDateTime,
    /// The name of the quote line the comment is about, as it read when written.
    pub anchor: Option<String>,
}

impl QuoteComment {
    pub fn from_customer(&self) -> bool {
        self.author_user_id.is_none()
    }
}

impl From<QuoteCommentRow> for QuoteComment {
    fn from(r: QuoteCommentRow) -> Self {
        Self {
            id: r.id,
            quote_id: r.quote_id,
            revision: r.revision,
            author_user_id: r.author_user_id,
            author_email: r.author_email,
            author_name: r.author_name,
            body: r.body,
            shared: r.shared,
            is_change_request: r.is_change_request,
            resolved_at: r.resolved_at,
            resolved_by: r.resolved_by,
            created_at: r.created_at,
            anchor: r.anchor,
        }
    }
}

#[derive(Debug, Clone)]
pub enum QuoteCommentAuthor {
    Member(UserId),
    /// A recipient of the quote, writing from the portal.
    Recipient {
        email: String,
    },
}

#[derive(Debug, Clone)]
pub struct QuoteCommentNew {
    pub quote_id: QuoteId,
    pub author: QuoteCommentAuthor,
    pub body: String,
    /// Visible to the customer in the portal; customer messages always are.
    pub shared: bool,
    pub is_change_request: bool,
    pub anchor: Option<String>,
}
