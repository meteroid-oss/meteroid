#import sys: inputs
#import "credit_note.typ": credit_note

#credit_note(
  inputs.lang,
  inputs.organization,
  inputs.customer,
  inputs.number,
  inputs.related_invoice_number,
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
  whitelabel: inputs.at("whitelabel", default: false)
)
