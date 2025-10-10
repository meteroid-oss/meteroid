use common_domain::ids::{CustomerId, TenantId};
use common_domain::ids::{InvoiceId, string_serde};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MeteroidMetadata {
    #[serde(with = "string_serde")]
    pub meteroid_invoice_id: InvoiceId,
    #[serde(with = "string_serde")]
    pub meteroid_tenant_id: TenantId, // todo: remove this field?
    #[serde(with = "string_serde")]
    pub meteroid_customer_id: CustomerId,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Period {
    /// The end date of this usage period.
    ///
    /// All usage up to and including this point in time is included.
    pub end: Option<i64>,

    /// The start date of this usage period.
    ///
    /// All usage after this point in time is included.
    pub start: Option<i64>,
}

/// differs from `stripe::InvoiceStatus` by containing also Deleted status
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Draft,
    Open,
    Paid,
    Uncollectible,
    Void,
    Deleted,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Invoice {
    pub id: String,
    /// The ID of the customer who will be billed.
    pub customer: Option<String>,
    #[serde(default)]
    pub metadata: MeteroidMetadata,
    pub status: Option<InvoiceStatus>,
    pub currency: Option<String>,
}

/// An enum representing the possible values of an `Invoice`'s `collection_method` field.
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CollectionMethod {
    ChargeAutomatically,
    SendInvoice,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize, Default)]
pub struct CreateInvoice<'a> {
    /// Controls whether Stripe will perform [automatic collection](https://stripe.com/docs/billing/invoices/workflow/#auto_advance) of the invoice.
    ///
    /// When `false`, the invoice's state will not automatically advance without an explicit action.
    pub auto_advance: Option<bool>,

    /// The currency to create this invoice in.
    ///
    /// Defaults to that of `customer` if not specified.
    pub currency: Option<&'a str>,

    /// Either `charge_automatically`, or `send_invoice`.
    ///
    /// When charging automatically, Stripe will attempt to pay this invoice using the default source attached to the customer.
    /// When sending an invoice, Stripe will email this invoice to the customer with payment instructions.
    /// Defaults to `charge_automatically`.
    pub collection_method: Option<CollectionMethod>,

    /// The number of days from when the invoice is created until it is due.
    ///
    /// Valid only for invoices where `collection_method=send_invoice`.
    pub days_until_due: Option<u32>,

    /// The ID of the customer who will be billed.
    pub customer: Option<&'a str>,

    /// Set of [key-value pairs](https://stripe.com/docs/api/metadata) that you can attach to an object.
    ///
    /// This can be useful for storing additional information about the object in a structured format.
    /// Individual keys can be unset by posting an empty value to them.
    /// All keys can be unset by posting an empty value to `metadata`.
    pub metadata: MeteroidMetadata,
}

#[skip_serializing_none]
#[derive(Clone, Debug, Serialize)]
pub struct CreateInvoiceItem<'a> {
    /// The integer amount in cents (or local equivalent) of the charge to be applied to the upcoming invoice.
    ///
    /// Passing in a negative `amount` will reduce the `amount_due` on the invoice.
    pub amount: Option<i64>,

    /// Three-letter [ISO currency code](https://www.iso.org/iso-4217-currency-codes.html), in lowercase.
    ///
    /// Must be a [supported currency](https://stripe.com/docs/currencies).
    pub currency: Option<&'a str>,

    /// The ID of the customer who will be billed when this invoice item is billed.
    pub customer: &'a str,

    /// An arbitrary string which you can attach to the invoice item.
    ///
    /// The description is displayed in the invoice for easy tracking.
    pub description: Option<&'a str>,

    /// The ID of an existing invoice to add this invoice item to.
    ///
    /// When left blank, the invoice item will be added to the next upcoming scheduled invoice.
    /// This is useful when adding invoice items in response to an invoice.created webhook.
    /// You can only add invoice items to draft invoices and there is a maximum of 250 items per invoice.
    pub invoice: Option<&'a str>,

    /// The period associated with this invoice item.
    ///
    /// When set to different values, the period will be rendered on the invoice.
    /// If you have [Stripe Revenue Recognition](https://stripe.com/docs/revenue-recognition) enabled, the period will be used to recognize and defer revenue.
    /// See the [Revenue Recognition documentation](https://stripe.com/docs/revenue-recognition/methodology/subscriptions-and-invoicing) for details.
    pub period: Option<Period>,
}

/// The resource representing a Stripe "`InvoiceItem`".
///
/// For more details see <https://stripe.com/docs/api/invoiceitems/object>
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct InvoiceItem {
    /// Unique identifier for the object.
    pub id: String,
}
