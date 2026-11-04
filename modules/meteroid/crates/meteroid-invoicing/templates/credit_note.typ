#import sys: inputs

// A value counts as set only when it is neither none nor empty. Cleared text fields
// can still reach us as "", and a section gated on `!= none` alone would then print
// its heading with nothing under it.
#let present(value) = value != none and value != ""

// Main credit note function with modern, clean design
#let credit_note(
  lang,
  organization,
  customer,
  number,
  related_invoice_number,
  related_invoice_date,
  issue_date,
  subtotal,
  tax_amount,
  total_amount,
  currency_code,
  currency_symbol,
  reason,
  memo,
  credit_type,
  refunded_amount,
  credited_amount,
  lines,
  tax_breakdown,
  translations,
  // Optional section flags
  show_tax_info: true,
  show_legal_info: true,
  show_footer_custom_info: true,
  whitelabel: false,
  debt_cancelled_amount: 0,
  promotional_credit_restored: 0,
  // Between the subtotal and the tax, in the note's sign: subtotal + both + tax = total.
  discount: 0,
  promotional_credit: 0,
) = {
  // Define color palette with named variables
  let color = (
    heading: rgb(25, 25, 25),
    text: rgb(55, 55, 55),
    accent: rgb(80, 80, 80),
    border: rgb(230, 230, 230),
    light_border: rgb(240, 240, 240),
    footer_text: rgb(120, 120, 120),
    date_text: rgb(130, 130, 130),
    white: rgb(255, 255, 255),
    subline_text: rgb(55, 55, 55),
    // Credit note specific - use a blue/teal accent
    credit_note_accent: rgb(59, 130, 246),
    refund_badge: rgb(239, 68, 68),
    credit_badge: rgb(34, 197, 94),
    debt_cancellation_badge: rgb(107, 114, 128),
  )

  // Amounts are decimals scaled by the renderer, the template only places the currency symbol
  let format_amount = (amount) => {
    if lang.starts-with("fr") {
      str(amount) + "\u{00A0}" + currency_symbol
    } else {
      currency_symbol + "\u{00A0}" + str(amount)
    }
  }

  // A summary adjustment carries its sign: on a negative note a reversed reduction adds back.
  let format_signed = (amount) => {
    if amount > 0 { "+" + format_amount(amount) } else { format_amount(amount) }
  }

  // Import custom font
  let inter = "inter"

  // Set page size and margins
  set page(
    paper: "a4",
    margin: (left: 36pt, right: 36pt, top: 50pt, bottom: 46pt),

    // Footer
    footer: context {
      v(8pt)
      grid(
        columns: (6fr, 1fr),
        if whitelabel == false {
          text(font: inter, size: 8pt, fill: color.footer_text, [
            #link("https://meteroid.com?utm_source=credit_note", [
              #box(baseline: 1pt, image("wordmark.svg", height: 8pt))
            ]) •
            #show link: underline
            #link("https://meteroid.com?utm_source=credit_note", [
              #text(font: inter, size: 8pt, fill: color.heading, [Billing automation for SaaS])
            ]) • #number • #format_amount(total_amount)
          ])
        } else {
          text(font: inter, size: 8pt, fill: color.footer_text, [
            #number • #format_amount(total_amount)
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
  set document(title: translations.credit_note_title + " " + number, date: datetime.today())

  // Define styles
  set text(font: inter, size: 9.5pt, fill: color.text)
  set heading(numbering: none)

  // Header layout
  grid(
    columns: (3fr, 3fr),
    column-gutter: 2pt,

    // Credit note title
    [
      #text(weight: "bold", size: 24pt, fill: color.heading, translations.credit_note_title)
    ],

    if organization.logo_src != none {
      align(right, image(organization.logo_src, height: 30pt, width: 150pt, fit: "contain"))
    }
  )

  v(16pt)

  grid(
    columns: (140pt, auto),
    row-gutter: 6pt,

    [#text(fill: color.accent, weight: "medium", translations.credit_note_number)],
    [#text(weight: "medium", number)],

    [#text(fill: color.accent, weight: "medium", translations.issue_date)],
    [#text(weight: "medium", issue_date)],

    [#text(fill: color.accent, weight: "medium", translations.related_invoice)],
    [#text(weight: "medium", translations.related_invoice_value)],

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

  v(24pt)

  // Company and client info
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

    // Credit To
    [
      #text(fill: color.heading, weight: "medium", size: 10pt, translations.credit_to)
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

    // Total credit amount and type
    [
      #align(right, [
        #text(size: 16pt, weight: "bold", fill: color.heading, [
          #format_amount(total_amount)
        ])
        #v(4pt)

        // Credit type badge
        #let type_text = if credit_type == "refund" {
          translations.refund
        } else if credit_type == "debt_cancellation" {
          translations.debt_cancellation
        } else {
          translations.credit_to_balance
        }

        #let type_color = if credit_type == "refund" {
          color.refund_badge
        } else if credit_type == "debt_cancellation" {
          color.debt_cancellation_badge
        } else {
          color.credit_badge
        }

        #box(
          fill: type_color.lighten(85%),
          radius: 4pt,
          inset: (x: 8pt, y: 4pt),
          text(fill: type_color, weight: "medium", size: 9pt, type_text)
        )

      ])
    ]
  )

  // Show reason if provided
  if reason != none {
    v(16pt)
    block(
      width: 100%,
      fill: color.light_border,
      radius: 4pt,
      inset: (x: 12pt, y: 8pt),
      [
        #text(fill: color.accent, weight: "medium", size: 9pt, translations.reason + ": ")
        #text(fill: color.text, reason)
      ]
    )
  }

  v(30pt)

  // Table header
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

  // Line items
  for (index, item) in lines.enumerate() {
    grid(
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
        #linebreak()
        #text(size: 8pt, fill: color.date_text, item.start_date + " → " + item.end_date)
      ],

      align(center, text(weight: "regular", if item.quantity != none { str(item.quantity) } else { "" })),

      align(right, text(weight: "regular", if item.unit_price != none { format_amount(item.unit_price) } else { "" })),

      align(right, text(weight: "regular", if item.vat_rate != none { str(item.vat_rate) + "%" } else { "" })),

      align(right, text(weight: "regular", format_amount(item.subtotal))),
    )

    // Sublines
    if item.sub_lines != none and item.sub_lines.len() > 0 {
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

    // Separator
    if index < lines.len() - 1 {
      v(3pt)
      line(length: 100%, stroke: 0.75pt + color.light_border)
      v(3pt)
    }
  }

  v(16pt)

  // Summary section
  grid(
    columns: (1fr, 1fr),
    column-gutter: 40pt,

    // Credit breakdown (left)
    align(left + top, [
      #line(length: 100%, stroke: 0pt)
      #v(12pt)

      #let effects = (
        (translations.amount_to_refund, refunded_amount),
        (translations.credited_amount, credited_amount),
        (translations.promotional_credit_restored, promotional_credit_restored),
        (translations.debt_cancelled_amount, debt_cancelled_amount),
      ).filter(effect => effect.at(1) > 0)

      #if effects.len() > 0 {
        grid(
          columns: (auto, auto),
          column-gutter: 20pt,
          row-gutter: 8pt,
          ..for (label, value) in effects {
            (
              text(fill: color.accent, label),
              text(weight: "medium", format_amount(value)),
            )
          }
        )
      }
    ]),

    // Totals section (right)
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

        ..if discount != 0 {
          (
            text(fill: color.accent, translations.discount),
            align(right, text(weight: "regular", format_signed(discount))),
          )
        },

        ..if promotional_credit != 0 {
          (
            text(fill: color.accent, translations.promotional_credit),
            align(right, text(weight: "regular", format_signed(promotional_credit))),
          )
        },

        ..if tax_breakdown.len() > 0 and calc.abs(tax_amount) > 0 {
          for tax_item in tax_breakdown {
            (
              text(fill: color.accent, tax_item.name + " " + str(tax_item.rate) + "%"),
              align(right, text(weight: "regular", format_amount(tax_item.amount))),
            )
          }
        },

        text(weight: "medium", size: 12pt, fill: color.heading, translations.total_credit),
        align(right, text(weight: "medium", size: 12pt, fill: color.heading, format_amount(total_amount))),
      )
    ])
  )

  v(30pt)

  // Tax info section
  if show_tax_info {
    block(
      width: 100%,
      [
        #line(length: 100%, stroke: 0.5pt + color.border)
        #v(16pt)

        #grid(
          columns: (1fr),

          [
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
            #let merchant_mention = tax_breakdown.map(item => item.at("exemption_reason", default: none)).find(present)

            // The mention itself always renders — a credit note must state its tax
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
            } else if calc.abs(tax_amount) > 0 {
              text(size: 9pt, translations.at("tax_included_resolved", default: "All prices include tax"))
            } else {
              text(size: 9pt, translations.at("no_tax_applied", default: "No tax applied"))
            }
          ]
        )
      ]
    )
  }

  // Trailing footer sections. The leading spacing is emitted only when at least one
  // of them has content, and weakly at that: on its own it is enough to spill a page
  // carrying nothing but the page footer, when the body ends near the bottom margin.
  let has_legal_number = show_legal_info and present(organization.legal_number)
  let has_legal_info = show_legal_info and present(organization.footer_legal)
  let has_footer_info = show_footer_custom_info and present(organization.footer_info)
  let has_exchange_rate = organization.exchange_rate != none and organization.accounting_currency_code != none and present(translations.at("exchange_rate_info", default: none))

  if has_legal_info or has_legal_number or has_footer_info or has_exchange_rate {
    v(16pt, weak: true)
  }

  // Legal information
  if has_legal_info or has_legal_number {
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
  if has_footer_info {
    v(16pt)
    text(size: 8pt, fill: color.footer_text, organization.footer_info)
  }

  // Exchange rate info
  if has_exchange_rate {
    v(8pt)
    text(size: 8pt, fill: color.footer_text, translations.exchange_rate_info)
  }
}
