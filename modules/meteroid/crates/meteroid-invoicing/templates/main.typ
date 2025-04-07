#import sys: inputs
#import "invoice.typ": invoice

#invoice(
  inputs.lang,
  inputs.organization,
  inputs.customer,
  inputs.number,
  inputs.issue_date,
  inputs.due_date,
  inputs.subtotal,
  inputs.tax_amount,
  inputs.tax_rate,
  inputs.total_amount,
  inputs.currency_code,
  inputs.currency_symbol,
  inputs.memo,
  inputs.payment_term,
  inputs.lines,
  inputs.translations,
  inputs.formatted_currency,
  pay_online_url: inputs.at("pay_online_url", default: none),
  footer_custom_message: inputs.at("footer_custom_message", default: none),
  payment_status: inputs.at("payment_status", default: "unpaid"),
  transactions: inputs.at("transactions", default: ()),
  payment_info: inputs.at("payment_info", default: none),
  show_payment_status: inputs.at("show_payment_status", default: true),
  show_payment_info: inputs.at("show_payment_info", default: true),
  show_terms: inputs.at("show_terms", default: true),
  show_tax_info: inputs.at("show_tax_info", default: true),
  show_legal_info: inputs.at("show_legal_info", default: true)
)
