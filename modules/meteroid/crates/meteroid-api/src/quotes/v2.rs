//! The parts of a quote the `quotes_full` gate covers: line presentation and discounts,
//! countersigners, signature level, CRM link, restricted customer actions, attached files. Core
//! applies them; writes that introduce or change them need the gate, writes that keep the stored
//! values do not.

use common_domain::ids::{AddOnId, PriceComponentId};
use meteroid_app::features::{Entitlements, QUOTES_FULL_NOT_IN_PLAN, gates};
use meteroid_grpc::meteroid::api::quotes::v1::{QuoteLineDetail, quote_line_detail::Target};
use meteroid_store::constants::Currencies;
use meteroid_store::domain::enums::QuoteSignatureLevelEnum;
use meteroid_store::domain::quote_document::discount_fee;
use meteroid_store::domain::quotes::{
    DetailedQuote, QuoteAddOnNew, QuoteAttachment, QuoteNew, QuotePriceComponentNew,
};
use meteroid_store::services::PendingMaterialization;
use rust_decimal::Decimal;
use serde_json::json;
use tonic::Status;

use common_domain::ids::OrganizationId;

pub(crate) fn parse_discount(value: &str) -> Result<Decimal, Status> {
    let discount: Decimal = value
        .trim()
        .parse()
        .map_err(|_| Status::invalid_argument(format!("Invalid discount '{value}'")))?;
    if discount <= Decimal::ZERO || discount > Decimal::ONE_HUNDRED {
        return Err(Status::invalid_argument(
            "A line discount is a percentage above 0 and at most 100",
        ));
    }
    Ok(discount)
}

/// Applies the request's line details to the resolved lines. A discounted line bills its own
/// discounted fee, no longer the catalog price.
pub(crate) fn apply_line_details(
    details: &[QuoteLineDetail],
    components: &mut [QuotePriceComponentNew],
    add_ons: &mut [QuoteAddOnNew],
    pending_addon_materializations: &mut Vec<PendingMaterialization>,
    currency: &str,
) -> Result<(), Status> {
    let precision = u32::from(Currencies::resolve_currency_precision(currency).unwrap_or(2));

    for detail in details {
        let discount = detail
            .discount_percent
            .as_deref()
            .filter(|d| !d.trim().is_empty())
            .map(parse_discount)
            .transpose()?;
        let description = detail.description.clone().filter(|d| !d.trim().is_empty());

        match &detail.target {
            Some(Target::PriceComponentId(_) | Target::ExtraComponentName(_)) => {
                let component = components
                    .iter_mut()
                    .find(|c| match &detail.target {
                        Some(Target::PriceComponentId(id)) => c
                            .price_component_id
                            .zip(id.parse::<PriceComponentId>().ok())
                            .is_some_and(|(a, b)| a == b),
                        Some(Target::ExtraComponentName(name)) => {
                            c.price_component_id.is_none() && &c.name == name
                        }
                        _ => false,
                    })
                    .ok_or_else(|| {
                        Status::invalid_argument("A line detail targets no line of the quote")
                    })?;
                component.description = description;
                component.sort_order = detail.sort_order;
                if let Some(discount) = discount {
                    let list = component.fee.clone();
                    component.fee = discount_fee(&list, discount, precision);
                    component.list_fee = Some(list);
                    component.discount_percent = Some(discount);
                    component.price_id = None;
                    component.is_override = true;
                }
            }
            Some(Target::AddOnId(id)) => {
                let add_on_id: AddOnId = id
                    .parse()
                    .map_err(|_| Status::invalid_argument("Invalid add_on_id"))?;
                let (index, add_on) = add_ons
                    .iter_mut()
                    .enumerate()
                    .find(|(_, a)| a.add_on_id == add_on_id)
                    .ok_or_else(|| {
                        Status::invalid_argument("A line detail targets no add-on of the quote")
                    })?;
                add_on.description = description;
                add_on.sort_order = detail.sort_order;
                if let Some(discount) = discount {
                    let list = add_on.fee.clone();
                    add_on.fee = discount_fee(&list, discount, precision);
                    add_on.list_fee = Some(list);
                    add_on.discount_percent = Some(discount);
                    add_on.price_id = None;
                    pending_addon_materializations.retain(|m| m.component_index != index);
                }
            }
            None => return Err(Status::invalid_argument("A line detail needs a target")),
        }
    }
    Ok(())
}

fn line_key(price_component_id: Option<PriceComponentId>, name: &str) -> String {
    price_component_id.map_or_else(|| format!("extra:{name}"), |id| format!("pc:{id}"))
}

/// What the line-editing gate covers, comparable between a request and the stored quote.
pub(crate) fn line_editing_fingerprint(
    components: impl Iterator<Item = (String, Option<String>, i32, Option<Decimal>)>,
    add_ons: impl Iterator<Item = (String, Option<String>, i32, Option<Decimal>)>,
    quote: &QuoteNewView,
) -> serde_json::Value {
    let lines: Vec<serde_json::Value> = components
        .chain(add_ons)
        .filter(|(_, description, sort_order, discount)| {
            description.is_some() || *sort_order != 0 || discount.is_some()
        })
        .map(|(key, description, sort_order, discount)| {
            json!([
                key,
                description,
                sort_order,
                discount.map(|d| d.normalize().to_string())
            ])
        })
        .collect();
    json!({
        "lines": lines,
        "display": serde_json::to_value(&quote.display_options).unwrap_or_default(),
    })
}

/// The gated quote-level fields, borrowed from a new or a stored quote.
pub(crate) struct QuoteNewView {
    pub display_options: meteroid_store::domain::quotes::QuoteDisplayOptions,
}

pub(crate) fn requested_line_editing(
    quote: &QuoteNew,
    components: &[QuotePriceComponentNew],
    add_ons: &[QuoteAddOnNew],
) -> serde_json::Value {
    line_editing_fingerprint(
        components.iter().map(|c| {
            (
                line_key(c.price_component_id, &c.name),
                c.description.clone(),
                c.sort_order,
                c.discount_percent,
            )
        }),
        add_ons.iter().map(|a| {
            (
                format!("ao:{}", a.add_on_id),
                a.description.clone(),
                a.sort_order,
                a.discount_percent,
            )
        }),
        &QuoteNewView {
            display_options: quote.display_options.clone(),
        },
    )
}

pub(crate) fn stored_line_editing(stored: Option<&DetailedQuote>) -> serde_json::Value {
    match stored {
        None => line_editing_fingerprint(
            std::iter::empty(),
            std::iter::empty(),
            &QuoteNewView {
                display_options: Default::default(),
            },
        ),
        Some(stored) => line_editing_fingerprint(
            stored.components.iter().map(|c| {
                (
                    line_key(c.price_component_id, &c.name),
                    c.description.clone(),
                    c.sort_order,
                    c.discount_percent,
                )
            }),
            stored.add_ons.iter().map(|a| {
                (
                    format!("ao:{}", a.add_on_id),
                    a.description.clone(),
                    a.sort_order,
                    a.discount_percent,
                )
            }),
            &QuoteNewView {
                display_options: stored.quote.display_options.clone(),
            },
        ),
    }
}

/// Refuses a write that introduces or changes a v2 value without `quotes_full`. `stored` is the
/// quote being edited, `None` on creation; keeping the stored values stays open.
pub(crate) async fn ensure_v2_allowed(
    resolver: &dyn Entitlements,
    org: OrganizationId,
    quote: &QuoteNew,
    components: &[QuotePriceComponentNew],
    add_ons: &[QuoteAddOnNew],
    stored: Option<&DetailedQuote>,
) -> Result<(), Status> {
    let stored_quote = stored.map(|s| &s.quote);
    let actions = (
        quote.allow_questions,
        quote.allow_change_requests,
        quote.allow_decline,
    );
    let changes = requested_line_editing(quote, components, add_ons) != stored_line_editing(stored)
        || quote.countersigners
            != stored_quote
                .map(|q| q.countersigners.clone())
                .unwrap_or_default()
        || (quote.signature_level == QuoteSignatureLevelEnum::Verified
            && stored_quote.map(|q| q.signature_level) != Some(QuoteSignatureLevelEnum::Verified))
        || (quote.hubspot_deal_id.is_some()
            && quote.hubspot_deal_id != stored_quote.and_then(|q| q.hubspot_deal_id.clone()))
        || adds_attachments(
            &quote.attachments,
            stored_quote.map_or(&[][..], |q| &q.attachments),
        )
        || (quote.template_id.is_some()
            && quote.template_id != stored_quote.and_then(|q| q.template_id))
        || (actions != (true, true, true)
            && Some(actions)
                != stored_quote
                    .map(|q| (q.allow_questions, q.allow_change_requests, q.allow_decline)));
    meteroid_app::features::ensure_change_allowed(
        resolver,
        org,
        gates::QUOTES_FULL,
        changes,
        QUOTES_FULL_NOT_IN_PLAN,
    )
    .await
}

/// Removing files keeps the others as stored, so a lapsed plan can still drop one.
fn adds_attachments(requested: &[QuoteAttachment], stored: &[QuoteAttachment]) -> bool {
    let mut kept = stored.iter().map(|a| a.id);
    !requested.iter().all(|a| kept.any(|id| id == a.id))
}

/// Accepts a HubSpot deal id or a deal's URL (`.../record/0-3/<id>`).
pub(crate) fn parse_hubspot_deal_id(value: &str) -> Result<Option<String>, Status> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let candidate = value
        .split(['/', '?', '#'])
        .filter(|s| !s.is_empty())
        .skip_while(|s| *s != "0-3")
        .nth(1)
        .unwrap_or(value);
    if candidate.chars().all(|c| c.is_ascii_digit()) && !candidate.is_empty() {
        Ok(Some(candidate.to_string()))
    } else {
        Err(Status::invalid_argument(
            "Enter a HubSpot deal id or the link of the deal",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deal_link_gives_its_id() {
        assert_eq!(
            parse_hubspot_deal_id("https://app-eu1.hubspot.com/contacts/1234/record/0-3/98765/")
                .unwrap(),
            Some("98765".to_string())
        );
        assert_eq!(
            parse_hubspot_deal_id(" 42 ").unwrap(),
            Some("42".to_string())
        );
        assert_eq!(parse_hubspot_deal_id("").unwrap(), None);
        assert!(parse_hubspot_deal_id("not-a-deal").is_err());
    }

    #[test]
    fn only_removing_attachments_keeps_the_rest_as_stored() {
        let file = |n: u128| QuoteAttachment {
            id: common_domain::ids::QuoteAttachmentId::from_const(uuid::Uuid::from_u128(n)),
            filename: format!("{n}.pdf"),
            content_type: "application/pdf".to_string(),
            size_bytes: 1,
            sha256: String::new(),
        };
        let stored = [file(1), file(2), file(3)];
        assert!(!adds_attachments(&stored, &stored));
        assert!(!adds_attachments(&[file(1), file(3)], &stored));
        assert!(!adds_attachments(&[], &stored));
        assert!(adds_attachments(&[file(3), file(1)], &stored), "reordered");
        assert!(adds_attachments(&[file(1), file(4)], &stored), "added");
        assert!(adds_attachments(&[file(1)], &[]));
    }

    #[test]
    fn a_discount_is_a_percentage() {
        assert!(parse_discount("0").is_err());
        assert!(parse_discount("100.5").is_err());
        assert_eq!(parse_discount("12.5").unwrap(), Decimal::new(125, 1));
    }
}
