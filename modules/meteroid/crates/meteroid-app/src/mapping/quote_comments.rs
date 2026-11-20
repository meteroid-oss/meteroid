use crate::conversions::{AsProtoOpt, ProtoConv};
use meteroid_grpc::meteroid::api::quotes::v1::QuoteComment as Proto;
use meteroid_store::domain::quote_comments::QuoteComment;

/// The portal sees who wrote, not the team's user ids or emails.
pub fn comment_to_proto(comment: &QuoteComment, for_portal: bool) -> Proto {
    Proto {
        id: comment.id.as_proto(),
        quote_id: comment.quote_id.as_proto(),
        revision: comment.revision,
        author_user_id: comment
            .author_user_id
            .filter(|_| !for_portal)
            .map(|id| id.as_proto()),
        author_email: comment
            .author_email
            .clone()
            .filter(|_| !for_portal || comment.from_customer()),
        author_name: comment.author_name.clone(),
        body: comment.body.clone(),
        shared: comment.shared,
        is_change_request: comment.is_change_request,
        resolved_at: comment.resolved_at.as_proto(),
        created_at: comment.created_at.as_proto(),
        from_customer: comment.from_customer(),
        anchor: comment.anchor.clone(),
    }
}
