#import sys: inputs

// A value counts as set only when it is neither none nor empty. Cleared text fields
// can still reach us as "", and a section gated on `!= none` alone would then print
// its heading with nothing under it.
#let present(value) = value != none and value != ""

// Main invoice function with modern, clean design
#let invoice(
  lang,
  organization,
  customer,
  number,
  purchase_order,
  issue_date,
  due_date,
  subtotal,
  tax_amount,
  total_amount,
  currency_code,
  currency_symbol,
  memo,
  payment_term,
  lines,
  coupons,
  discount,
  tax_breakdown,
  translations,
  pay_online_url: none,
  payment_status: "unpaid",  // "paid", "partially_paid", or "unpaid"
  transactions: (),  // list of transactions
  payment_info: none,  // key-value pairs for payment information
  // Optional section flags
  show_payment_status: true,  // Show payment status section
  show_payment_info: true,   // Show payment information section
  show_terms: true,          // Show payment terms section
  show_tax_info: true,       // Show tax information section
  show_legal_info: true,     // Show legal information section
  show_footer_custom_info: true,   // Show footer custom information section
  whitelabel: false,         // Hide meteroid branding
  parent_invoice_number: none,     // Set when this is a corrective invoice
  parent_invoice_date: none,       // Date of the original invoice
  annexes: (),                     // list of appendices rendered after the invoice body
  tax_inclusive: false,            // Lines and subtotal are quoted tax-included; tax is listed as included
  total_excl_tax: none,            // Net total shown above the included tax on a tax-inclusive document
  promotional_credit: 0,           // Promotional credit drawn before tax, like a discount
  prepaid_amount: 0,               // Customer credit applied at issuance, deducted from the total
  amount_due: none,                // total_amount - prepaid_amount; defaults to total_amount
  prepaid_credit_purchase: false,  // Sells currency credit, taxed when the credit is used
) = {
  let amount_due = if amount_due == none { total_amount } else { amount_due }
  // Credit, or a note, covered it all: no due date to state and nothing to pay online.
  let nothing_due = amount_due <= 0

  // Define color palette with named variables
  let color = (
    heading: rgb(25, 25, 25),
    text: rgb(55, 55, 55),
    accent: rgb(80, 80, 80),
    border: rgb(230, 230, 230),
    light_border: rgb(240, 240, 240),
    footer_text: rgb(120, 120, 120),
    date_text: rgb(130, 130, 130),
    button: rgb(50, 125, 230),
    white: rgb(255, 255, 255),
    subline_text: rgb(55, 55, 55),
    // Status colors
    paid: rgb(76, 175, 80),
    partially_paid: rgb(255, 152, 0),
    unpaid: rgb(244, 67, 54),
  )

  // Amounts are decimals scaled by the renderer, the template only places the currency symbol
  let format_amount = (amount) => {
    if lang.starts-with("fr") {
      str(amount) + "\u{00A0}" + currency_symbol
    } else {
      currency_symbol + "\u{00A0}" + str(amount)
    }
  }
  let due_summary = if nothing_due {
    translations.at("nothing_to_pay", default: "Nothing to pay")
  } else {
    format_amount(amount_due) + " " + translations.due_label + " " + due_date
  }


  // Import custom font
  let inter = "inter"

  // Set page size and margins - wider margins for more whitespace
  set page(
    paper: "a4",
    margin: (left: 36pt, right: 36pt, top: 50pt, bottom: 46pt),

    // Redesigned footer with two parts and merged message
    footer: context {
      v(8pt)
      grid(
        columns: (6fr, 1fr),
        // Custom message + invoice info (left)
        if whitelabel == false {
          text(font: inter, size: 8pt, fill: color.footer_text, [
            #link("https://meteroid.com?utm_source=invoice", [
              #box(baseline: 1pt, image("wordmark.svg", height: 8pt))
            ]) •
            #show link: underline
            #link("https://meteroid.com?utm_source=invoice", [
              #text(font: inter, size: 8pt, fill: color.heading, [Billing automation for SaaS])
            ]) • #number • #due_summary
          ])
        } else {
          text(font: inter, size: 8pt, fill: color.footer_text, [
            #number • #due_summary
          ])
        },

        // Page number (right)
        align(right, text(font: inter, size: 8pt, fill: color.footer_text, [
          Page #counter(page).display() of #counter(page).final().first()
        ]))
      )
      v(30pt)
    }
  )

  // Set document metadata
  set document(title: translations.invoice_title + " " + number, date: datetime.today())

  // Define styles - using the Inter variable font
  set text(font: inter, size: 9.5pt, fill: color.text)
  set heading(numbering: none)

  // Start with clean header layout
  grid(
    columns: (3fr, 3fr),
    column-gutter: 2pt,

    // Invoice title and info
    [
      #text(weight: "bold", size: 24pt, fill: color.heading, translations.invoice_title)

    ],

    if organization.logo_src != none {
      align(right, image(organization.logo_src, height: 30pt, width: 150pt, fit: "contain"))
    }
  )

  v(16pt)

  grid(
    columns: (120pt, auto),
    row-gutter: 6pt,

    [#text(fill: color.accent, weight: "medium", translations.invoice_number)],
    [#text(weight: "medium", number)],

    [#text(fill: color.accent, weight: "medium", translations.issue_date)],
    [#text(weight: "medium", issue_date)],

    [#text(fill: color.accent, weight: "medium", translations.due_date)],
    [#text(weight: "medium", due_date)],

    ..if purchase_order != none {
      (
        [#text(fill: color.accent, weight: "medium", translations.purchase_order)],
        [#text(weight: "medium", purchase_order)],
      )
    },

    ..if organization.tax_id != none {
      (
        [#text(fill: color.accent, weight: "medium", translations.seller_tax_label)],
        [#text(weight: "medium", organization.tax_id)],
      )
    },

    ..if customer.tax_id != none {
      (
        [#text(fill: color.accent, weight: "medium", translations.customer_tax_label)],
        [#text(weight: "medium", customer.tax_id)],
      )
    },

    ..if present(customer.legal_number) {
      (
        [#text(fill: color.accent, weight: "medium", translations.at("customer_registration", default: "Customer registration number"))],
        [#text(weight: "medium", customer.legal_number)],
      )
    },
  )

  // Legal notice for corrective invoices
  if parent_invoice_number != none {
    v(16pt)
    block(
      width: 100%,
      fill: color.light_border,
      radius: 4pt,
      inset: (x: 12pt, y: 8pt),
      text(fill: color.heading, weight: "medium", size: 9.5pt, translations.cancels_and_replaces)
    )
  }

  v(40pt)

  // Company and client info in a modern horizontal layout
  grid(
    columns: (3fr, 3fr, 5fr),
    column-gutter: 12pt,

    // From (Organization)
    [
      #text(fill: color.heading, weight: "medium", size: 10pt, organization.name)
      #v(6pt)
      #text(fill: color.accent, [
        #if organization.address.line1 != none and organization.address.line1 != "" [#organization.address.line1 #linebreak()]
        #if organization.address.line2 != none and organization.address.line2 != "" [#organization.address.line2 #linebreak()]
        #if organization.address.zipcode != none and organization.address.zipcode != "" [#organization.address.zipcode]
        #if organization.address.city != none and organization.address.city != "" [#organization.address.city #linebreak()]
        #if organization.address.country != none [#organization.address.country #linebreak()]
        #if organization.email != none [#organization.email]
      ])
    ],

    // Bill To
    [
      #text(fill: color.heading, weight: "medium", size: 10pt, translations.bill_to)
      #v(6pt)
      #text(fill: color.accent, [
        #customer.name #linebreak()
        #if customer.address.line1 != none and customer.address.line1 != "" [#customer.address.line1 #linebreak()]
        #if customer.address.line2 != none and customer.address.line2 != "" [#customer.address.line2 #linebreak()]
        #if (customer.address.zipcode != none and customer.address.zipcode != "") or (customer.address.city != none and customer.address.city != "") [
          #if customer.address.zipcode != none and customer.address.zipcode != "" [#customer.address.zipcode ]
          #if customer.address.city != none and customer.address.city != "" [#customer.address.city]
          #linebreak()
        ]
        #if customer.address.country != none and customer.address.country != "" [#customer.address.country #linebreak()]
        #if customer.email != none and customer.email != "" [#customer.email]
      ])
    ],

    // Amount due section with prominent display
    [
      #align(right, [
        #if nothing_due {
          text(size: 16pt, weight: "bold", fill: color.heading, translations.at("nothing_to_pay", default: "Nothing to pay"))
        } else [
          #text(size: 16pt, weight: "bold", fill: color.heading, [
            #format_amount(amount_due)
          ])
          #text(size: 12pt, weight: "bold", fill: color.heading, [
            #translations.due_label #due_date
          ])
        ]


        #if memo != none {
          text(fill: color.accent, [
            #memo
          ])
        }

        #v(4pt)

        // Add payment button with url link if provided
        #if pay_online_url != none and not nothing_due {
          link(pay_online_url,
            box(
              fill: color.button,
              radius: 4pt,
              inset: (x: 16pt, y: 8pt),
              text(fill: color.white, weight: "medium", size: 10pt, translations.pay_online)
            )
          )
        } else {

        }
      ])
    ]
  )

  v(30pt)



  // Simple table header with subtle styling and smaller font
  grid(
    columns: (3.6fr, 0.85fr, 1.5fr, 0.8fr, 1.25fr),
    column-gutter: 2pt,
    row-gutter: 0pt,

    text(weight: "medium", fill: color.accent, size: 8pt, translations.description),
    align(center, text(weight: "medium", fill: color.accent, size: 8pt, translations.quantity)),
    align(right, text(weight: "medium", fill: color.accent, size: 8pt, translations.unit_price)),
    align(right, text(weight: "medium", fill: color.accent, size: 8pt, translations.tax_rate)),
    align(right, text(weight: "medium", fill: color.accent, size: 8pt, translations.amount)),
  )

  line(length: 100%, stroke: 1pt + color.border)
  v(4pt)

  // Line items with compact styling and improved sublines
  for (index, item) in lines.enumerate() {
    // Plan grouping header: shown for consolidated invoices when the plan group changes.
    if item.group_label != none and (index == 0 or lines.at(index - 1).group_label != item.group_label) {
      if index > 0 {
        v(6pt)
      }
      block(width: 100%, text(weight: "bold", fill: color.heading, size: 9.5pt, item.group_label))
      v(3pt)
    }

    block(breakable: false, width: 100%, [
      #grid(
        columns: (3.6fr, 0.85fr, 1.5fr, 0.8fr, 1.25fr),
        column-gutter: 6pt,
        row-gutter: 0pt,

        [
          #text(weight: "medium", fill: color.heading, item.name)
          #if item.at("is_prorated", default: false) [
            #text(size: 8pt, style: "italic", fill: color.accent, " (" + translations.prorated + ")")
          ]
          #if item.description != none [
            #text(size: 9pt, fill: color.accent, item.description)
          ]
          #if item.at("credit", default: none) != none [
            #linebreak()
            #text(size: 8pt, fill: color.accent, item.credit.consumed + " " + translations.credits_used + " · " + item.credit.covered + " " + translations.credits_covered)
          ]
          #linebreak()
          #text(size: 8pt, fill: color.date_text, item.start_date + " → " + item.end_date)
        ],

        align(center, text(weight: "regular", if item.quantity != none { str(item.quantity) } else { "" })),

        align(right, text(weight: "regular", if item.unit_price != none { format_amount(item.unit_price) } else { "" })),

        align(right, text(weight: "regular", if item.vat_rate != none { str(item.vat_rate) + "%" } else { "" })),

        align(right, text(weight: "regular", format_amount(item.subtotal))),
      )

      // Add sublines if they exist
      #if item.sub_lines != none and item.sub_lines.len() > 0 {
        block(
          width: 100%,
          radius: 3pt,
          inset: (x: 3pt, y: 2pt),
          [
            #for (sub_index, sub_item) in item.sub_lines.enumerate() {
              grid(
                columns: (3.6fr, 0.85fr, 1.5fr, 0.8fr, 1.25fr),
                column-gutter: 6pt,
                row-gutter: 0pt,

                [
                  #box(width: 12pt, [])
                  #text(size: 8.5pt, fill: color.accent, [
                    #sub_item.name
                  ])
                ],

                align(center, text(size: 8.5pt, fill: color.accent, if sub_item.quantity != none { str(sub_item.quantity) } else { "" })),

                align(right, text(size: 8.5pt, fill: color.accent, if sub_item.unit_price != none { format_amount(sub_item.unit_price) } else { "" })),

                [],

                align(right, text(size: 8.5pt, fill: color.accent, format_amount(sub_item.total))),
              )
            }
          ]
        )
      }

      // Add minimal spacing between items
      #if index < lines.len() - 1 {
        v(3pt)
        line(length: 100%, stroke: 0.75pt + color.light_border)
      }
    ])

    if index < lines.len() - 1 {
      v(3pt)
    }
  }

  v(16pt)

  // Summary section aligned right with payment status on the left
  block(breakable: false, width: 100%, grid(
    columns: (1fr, 1fr),
    column-gutter: 40pt,

    // Payment status section (LEFT of summary) - Only shown if payment_status is not "unpaid" and show_payment_status is true
    if show_payment_status and payment_status != "unpaid" {
      align(left + top, [
        #line(length: 100%, stroke: 0pt)
        #v(12pt)

        // Payment status with badge
        #grid(
          columns: (auto, 1fr),
          column-gutter: 10pt,

          [
             #box(
              baseline: 10pt,
               text(fill: color.heading, weight: "medium", size: 10pt, translations.payment_status)
            )

          ],

          // Status badge - moved to right side and vertically aligned
          align(right + horizon, [
            // Status badge
            #let status_text = if payment_status == "paid" {
              translations.payment_status_paid
            } else if payment_status == "partially_paid" {
              translations.at("payment_status_partially_paid", default: "Partially Paid")
            } else {
              translations.at("payment_status_unpaid", default: "Unpaid")
            }

            #let status_color = if payment_status == "paid" {
              color.paid
            } else if payment_status == "partially_paid" {
              color.partially_paid
            } else {
              color.unpaid
            }

            #box(
              fill: status_color.lighten(85%),
              radius: 4pt,
              inset: (x: 8pt, y: 3pt),
              baseline: -0pt,
              text(fill: status_color, weight: "medium", size: 8.5pt, status_text)
            )
          ])
        )
        #v(2pt)

        // Transaction list - replaces hardcoded values
        #if transactions.len() > 0 {
          grid(
            columns: (auto, auto, auto),
            rows: (auto, ..range(transactions.len()).map(_ => auto)),
            column-gutter: 15pt,
            row-gutter: 8pt,

            // Header row
            [#text(fill: color.accent, weight: "medium", translations.payment_method)],
            [#text(fill: color.accent, weight: "medium", translations.payment_date)],
            [#text(fill: color.accent, weight: "medium", translations.payment_amount)],

            // Transaction rows
            ..for transaction in transactions {
              (
                [#transaction.method],
                [#transaction.date],
                [#format_amount(transaction.amount)],
              )
            }
          )
        } else {
          text(fill: color.accent, translations.at("no_transactions", default: "No payments received"))
        }
      ])
    } else {
      []
    },

    // Totals section (RIGHT)
    align(right, [
      #line(length: 100%, stroke: 1pt + color.border)
      #v(12pt)

      #grid(
        columns: (120pt, 80pt),
        rows: (auto, auto, auto),
        row-gutter: 8pt,
        column-gutter: 10pt,

        text(fill: color.accent, translations.subtotal),
        align(right, text(weight: "regular", format_amount(subtotal))),

        // Show coupon line items if available, otherwise show generic discount
        ..if coupons.len() > 0 {
          for coupon in coupons {
            (
              text(fill: color.accent, coupon.name),
              align(right, text(weight: "regular", sym.minus + format_amount(coupon.total))),
            )
          }
        } else if discount > 0 {
          (
            text(fill: color.accent, translations.discount),
            align(right, text(weight: "regular", sym.minus + format_amount(discount))),
          )
        },

        ..if promotional_credit > 0 {
          (
            text(fill: color.accent, translations.at("promotional_credit", default: "Promotional credit")),
            align(right, text(weight: "regular", sym.minus + format_amount(promotional_credit))),
          )
        },

        // Tax-inclusive: the quoted subtotal less the discount and promotional credit already
        // is the total, so the net total gets its own row and each tax row is marked as
        // included, not added.
        ..if tax_inclusive and total_excl_tax != none and tax_amount > 0 {
          (
            text(fill: color.accent, translations.at("total_excl_tax", default: "Total excluding tax")),
            align(right, text(weight: "regular", format_amount(total_excl_tax))),
          )
        },

        ..if tax_breakdown.len() > 0 and tax_amount > 0 {
          // Show tax breakdown for any non-zero tax
          for tax_item in tax_breakdown {
            (
              text(fill: color.accent, tax_item.name + " " + str(tax_item.rate) + "%" + if tax_inclusive { " (" + translations.at("tax_row_included", default: "included") + ")" } else { "" }),
              align(right, text(weight: "regular", format_amount(tax_item.amount))),
            )
          }
        },

        ..if prepaid_amount > 0 {
          (
            text(fill: color.accent, translations.at("total", default: "Total")),
            align(right, text(weight: "regular", format_amount(total_amount))),
            text(fill: color.accent, translations.at("credits_applied", default: "Credits applied")),
            align(right, text(weight: "regular", sym.minus + format_amount(prepaid_amount))),
            text(weight: "medium", size: 12pt, fill: color.heading, translations.amount_due),
            align(right, text(weight: "medium", size: 12pt, fill: color.heading, format_amount(amount_due))),
          )
        } else {
          (
            text(weight: "medium", size: 12pt, fill: color.heading, translations.total_due),
            align(right, text(weight: "medium", size: 12pt, fill: color.heading, format_amount(total_amount))),
          )
        },
      )
    ])
  ))

  v(30pt)

  // Add payment information section if provided and enabled
  if show_payment_info and payment_info != none {
    block(
      breakable: false,
      width: 100%,
      [
        #line(length: 100%, stroke: 0.5pt + color.border)
        #v(16pt)

        #text(fill: color.heading, weight: "medium", size: 10pt, translations.at("payment_info_title", default: "PAYMENT INFORMATION"))
        #v(4pt)



        #grid(
          columns: (1fr, 4fr),
          column-gutter: 10pt,
          row-gutter: 8pt,

          // Dynamic rows for payment information
          ..for (key, value) in payment_info.pairs() {
            (
            [#text(fill: color.heading, weight: "medium", key)],
            [#text(weight: "regular", value)],
            )
          }
        )

        #v(16pt)
      ]
    )
  }

  // Payment terms and tax info - only if enabled
  if show_terms or show_tax_info {
    block(
      breakable: false,
      width: 100%,
      [
        #line(length: 100%, stroke: 0.5pt + color.border)
        #v(16pt)

        // Terms and tax info
        #grid(
          columns: (1fr, 1fr),

          if show_terms [
            #text(fill: color.heading, weight: "medium", size: 10pt, translations.payment_terms_title)
            #v(4pt)
            #text(size: 9pt, translations.payment_terms_text)
          ] else [],

          if show_tax_info [
            #text(fill: color.heading, weight: "medium", size: 10pt, translations.tax_info_title)
            #v(4pt)

            #let has_reverse_charge = tax_breakdown.any(item => item.at("exemption_type", default: none) == "reverse_charge")
            #let has_export = tax_breakdown.any(item => item.at("exemption_type", default: none) == "export")
            // Out of scope without leaving the seller's territory (Guyane, Mayotte):
            // the same headline as an export, but its own citation, CGI art. 294-1.
            #let has_no_vat_territory = tax_breakdown.any(item => item.at("exemption_type", default: none) == "no_vat_territory")
            #let has_tax_exempt = tax_breakdown.any(item => item.at("exemption_type", default: none) == "tax_exempt")

            // The merchant's own legal mention for the exemption, where one is
            // configured. It replaces the derived statutory headline AND citation —
            // the precedence the EN 16931 mapping gives BT-120 — so the PDF and the
            // embedded XML never state two different tax positions on one document.
            // Only the determinate exemption branches consult it: on a taxed or
            // undetermined document the XML carries no BT-120 for it to mirror.
            #let merchant_mention = tax_breakdown.map(item => item.at("exemption_reason", default: none)).find(present)

            // The mention itself always renders — an invoice must state its tax
            // treatment. Only the supporting citation is dropped when absent, so a
            // missing bundle key leaves no dangling blank line under the heading.
            #let citation = (footnote) => {
              if present(footnote) {
                linebreak()
                text(size: 8pt, fill: color.footer_text, footnote)
              }
            }

            #if (has_reverse_charge or has_export or has_no_vat_territory or has_tax_exempt) and merchant_mention != none {
              text(size: 9pt, merchant_mention)
            } else if has_reverse_charge {
              text(size: 9pt, translations.tax_reverse_charge)
              citation(translations.at("reverse_charge_notice", default: ""))
            } else if has_export {
              text(size: 9pt, translations.at("vat_not_applicable_legal", default: "VAT not applicable"))
              citation(translations.at("export_notice_resolved", default: ""))
            } else if has_no_vat_territory {
              text(size: 9pt, translations.at("vat_not_applicable_legal", default: "VAT not applicable"))
              citation(translations.at("no_vat_territory_notice_resolved", default: ""))
            } else if has_tax_exempt {
              // An exemption is not an out-of-scope supply: its own headline, or a
              // document labels an exempt supply as one the tax simply does not reach.
              text(size: 9pt, translations.at("vat_exempt_legal", default: "Tax exempt"))
              citation(translations.at("vat_exempt_notice_resolved", default: ""))
            } else if tax_amount > 0 {
              text(size: 9pt, translations.at("tax_included_resolved", default: "All prices include tax"))
            } else {
              text(size: 9pt, translations.at("no_tax_applied", default: "No tax applied"))
            }

          ] else []
        )
      ]
    )
  }

  // Shown whatever the section flags: the customer is told when the tax is charged.
  if prepaid_credit_purchase {
    v(8pt, weak: true)
    text(size: 8pt, fill: color.footer_text, translations.at("prepaid_credit_tax_note", default: ""))
  }

  // Legal, footer, and exchange rate info — kept together to avoid orphan breaks.
  // Emitted only when at least one of them has content: an empty block plus its
  // leading spacing is enough to spill a page carrying nothing but the page footer,
  // when the body happens to end near the bottom margin.
  let has_legal_number = show_legal_info and present(organization.legal_number)
  let has_legal_info = show_legal_info and present(organization.footer_legal)
  let has_footer_info = show_footer_custom_info and present(organization.footer_info)
  let has_exchange_rate = organization.exchange_rate != none and organization.accounting_currency_code != none and present(translations.at("exchange_rate_info", default: none))

  if has_legal_info or has_legal_number or has_footer_info or has_exchange_rate {
    // Weak, so it collapses rather than pushing a page when the block breaks over.
    v(16pt, weak: true)

    block(breakable: false, width: 100%, [
      // Legal information
      #if has_legal_info or has_legal_number {
        grid(
          columns: (1fr),
          [
            #text(fill: color.heading, weight: "medium", size: 10pt, translations.legal_info)
            #v(4pt)
            #if has_legal_info {
              text(size: 8pt, fill: color.footer_text, organization.footer_legal)
            }

            #if has_legal_number {
              v(4pt)
              text(size: 8pt, fill: color.footer_text, [
                #translations.at("company_registration", default: "Registration"): #organization.legal_number
              ])
            }
          ]
        )
      }

      // Footer custom information
      #if has_footer_info {
        v(16pt)
        text(size: 8pt, fill: color.footer_text, organization.footer_info)
      }

      // Exchange rate info
      #if has_exchange_rate {
        v(8pt)
        text(size: 8pt, fill: color.footer_text, translations.exchange_rate_info)
      }
    ])
  }

  // Annexes: each appendix starts on a fresh page and renders a (possibly
  // multi-page) detail table. Grouping is display-only — rows are listed as-is.
  for annex in annexes {
    pagebreak(weak: true)

    text(weight: "bold", size: 16pt, fill: color.heading, annex.title)
    v(6pt)

    if annex.at("description", default: none) != none {
      text(size: 9pt, fill: color.accent, annex.description)
      v(8pt)
    }

    let col_count = annex.columns.len()

    // Tableless annexes (e.g. static text) render just the title + description above.
    if col_count > 0 {
      // Header cells.
      let header_cells = annex.columns.map(c =>
        table.cell(fill: color.light_border, text(weight: "medium", size: 8.5pt, fill: color.heading, c))
      )

      // Body: group heading rows (spanning all columns) interleaved with data rows.
      let body_cells = ()
      for group in annex.groups {
        if group.at("heading", default: none) != none {
          body_cells.push(
            table.cell(colspan: col_count, fill: color.light_border.lighten(40%),
              text(weight: "medium", size: 9pt, fill: color.heading, group.heading))
          )
        }
        for row in group.rows {
          let emphasized = row.at("emphasis", default: false)
          for cell in row.cells {
            if emphasized {
              body_cells.push(table.cell(
                fill: color.light_border,
                text(weight: "bold", size: 8.5pt, fill: color.heading, cell),
              ))
            } else {
              body_cells.push(text(size: 8.5pt, fill: color.text, cell))
            }
          }
        }
      }

      table(
        columns: col_count,
        inset: (x: 6pt, y: 4pt),
        align: left,
        stroke: 0.5pt + color.border,
        ..header_cells,
        ..body_cells,
      )
    }
  }
}

