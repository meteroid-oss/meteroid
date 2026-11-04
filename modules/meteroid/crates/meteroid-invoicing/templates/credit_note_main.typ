#import sys: inputs
#import "credit_note.typ": credit_note

// The structured credit note travels inside the PDF, as on an invoice.
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

#credit_note(
  inputs.lang,
  inputs.organization,
  inputs.customer,
  inputs.number,
  inputs.related_invoice_number,
  inputs.related_invoice_date,
  inputs.issue_date,
  inputs.subtotal,
  inputs.tax_amount,
  inputs.total_amount,
  inputs.currency_code,
  inputs.currency_symbol,
  inputs.reason,
  inputs.memo,
  inputs.credit_type,
  inputs.refunded_amount,
  inputs.credited_amount,
  inputs.lines,
  inputs.tax_breakdown,
  inputs.translations,
  show_tax_info: inputs.at("show_tax_info", default: true),
  show_legal_info: inputs.at("show_legal_info", default: true),
  show_footer_custom_info: inputs.at("show_footer_custom_info", default: true),
  whitelabel: inputs.at("whitelabel", default: false),
  debt_cancelled_amount: inputs.at("debt_cancelled_amount", default: 0),
  promotional_credit_restored: inputs.at("promotional_credit_restored", default: 0),
  discount: inputs.at("discount", default: 0),
  promotional_credit: inputs.at("promotional_credit", default: 0),
)
