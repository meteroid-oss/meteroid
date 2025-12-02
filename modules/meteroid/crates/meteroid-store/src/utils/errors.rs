use crate::errors::StoreError;
use error_stack::Report;

/// Formats the full error chain from an error_stack Report.
/// This captures all context/causes, not just the top-level error message.
pub fn format_error_chain(err: &Report<StoreError>) -> String {
    let mut messages = Vec::new();

    // Get the root error
    if let Some(store_error) = err.downcast_ref::<StoreError>() {
        messages.push(store_error.to_string());
    }

    // Collect all context messages from the chain
    for frame in err.frames() {
        // Try to get any attached string messages
        if let Some(msg) = frame.downcast_ref::<String>() {
            messages.push(msg.clone());
        } else if let Some(msg) = frame.downcast_ref::<&str>() {
            messages.push((*msg).to_string());
        }
    }

    if messages.is_empty() {
        // Fallback to debug format if we couldn't extract messages
        format!("{:?}", err)
    } else {
        messages.join(" -> ")
    }
}
