#import sys: inputs
#import "invoice.typ": invoice

// Factur-X / ZUGFeRD: the XML is an associated file of the PDF, not a separate
// document. `relationship` is what tells a receiver the attachment *is* the
// invoice; without it the file is found and ignored.
#let einvoice = inputs.at("einvoice", default: none)
#if einvoice != none {
  pdf.attach(
    einvoice.filename,
    einvoice.data,
    relationship: einvoice.relationship,
    mime-type: "application/xml",
    description: einvoice.description,
  )
}

#invoice(
  inputs.lang,
  inputs.organization,
  inputs.customer,
  inputs.number,
  inputs.purchase_order,
  inputs.issue_date,
  inputs.due_date,
  inputs.subtotal,
  inputs.tax_amount,
  inputs.total_amount,
  inputs.currency_code,
  inputs.currency_symbol,
  inputs.memo,
  inputs.payment_term,
  inputs.lines,
  inputs.coupons,
  inputs.discount,
  inputs.tax_breakdown,
  inputs.translations,
  pay_online_url: inputs.at("pay_online_url", default: none),
  payment_status: inputs.at("payment_status", default: "unpaid"),
  transactions: inputs.at("transactions", default: ()),
  payment_info: inputs.at("payment_info", default: none),
  show_payment_status: inputs.at("show_payment_status", default: true),
  show_payment_info: inputs.at("show_payment_info", default: true),
  show_terms: inputs.at("show_terms", default: true),
  show_tax_info: inputs.at("show_tax_info", default: true),
  show_legal_info: inputs.at("show_legal_info", default: true),
  show_footer_custom_info: inputs.at("show_footer_custom_info", default: true),
  parent_invoice_number: inputs.at("parent_invoice_number", default: none),
  parent_invoice_date: inputs.at("parent_invoice_date", default: none),
  annexes: inputs.at("annexes", default: ()),
  tax_inclusive: inputs.at("tax_inclusive", default: false),
  total_excl_tax: inputs.at("total_excl_tax", default: none),
  promotional_credit: inputs.at("promotional_credit", default: 0),
  prepaid_amount: inputs.at("prepaid_amount", default: 0),
  prepaid_credit_purchase: inputs.at("prepaid_credit_purchase", default: false),
  amount_due: inputs.at("amount_due", default: none),
)
