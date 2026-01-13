#import sys: inputs

// Main credit note function with modern, clean design
#let credit_note(
  lang,
  organization,
  customer,
  number,
  related_invoice_number,
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
  )

  // Format currency values
  let format_amount = (amount) => {
    let formatted = calc.round(amount, digits: 2)
    if lang.starts-with("fr") {
      str(formatted) + " " + currency_symbol
    } else {
      currency_symbol + " " + str(formatted)
    }
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
    } else {
      align(right, image("logo.png", width: 30pt))
    }
  )

  v(16pt)

  grid(
    columns: (140pt, auto),
    rows: (auto, auto, auto, auto),
    row-gutter: 6pt,

    [#text(fill: color.accent, weight: "medium", translations.credit_note_number)],
    [#text(weight: "medium", number)],

    [#text(fill: color.accent, weight: "medium", translations.issue_date)],
    [#text(weight: "medium", issue_date)],

    [#text(fill: color.accent, weight: "medium", translations.related_invoice)],
    [#text(weight: "medium", related_invoice_number)],

    if organization.tax_id != none [
      #text(fill: color.accent, weight: "medium", translations.vat_id)
    ] else [],

    if organization.tax_id != none [
      #text(weight: "medium", organization.tax_id)
    ] else [],
  )

  v(40pt)

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
          translations.refunded
        } else {
          translations.credit_to_balance
        }

        #let type_color = if credit_type == "refund" {
          color.refund_badge
        } else {
          color.credit_badge
        }

        #box(
          fill: type_color.lighten(85%),
          radius: 4pt,
          inset: (x: 8pt, y: 4pt),
          text(fill: type_color, weight: "medium", size: 9pt, type_text)
        )

        #v(8pt)

        #if memo != none {
          text(fill: color.accent, [
            #memo
          ])
        }
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
    columns: (4fr, 1fr, 1fr, 0.8fr, 1.2fr),
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
      columns: (4fr, 1fr, 1fr, 0.8fr, 1.2fr),
      column-gutter: 6pt,
      row-gutter: 0pt,

      [
        #text(weight: "medium", fill: color.heading, item.name)
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
              columns: (4fr, 1fr, 1fr, 0.8fr, 1.2fr),
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

      #if refunded_amount > 0 or credited_amount > 0 {
        grid(
          columns: (auto, auto),
          column-gutter: 20pt,
          row-gutter: 8pt,

          if refunded_amount > 0 [
            #text(fill: color.accent, translations.refunded_amount)
          ],
          if refunded_amount > 0 [
            #text(weight: "medium", format_amount(refunded_amount))
          ],

          if credited_amount > 0 [
            #text(fill: color.accent, translations.credited_amount)
          ],
          if credited_amount > 0 [
            #text(weight: "medium", format_amount(credited_amount))
          ],
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

        ..if tax_breakdown.len() > 0 and tax_amount > 0 {
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
            #let has_tax_exempt = tax_breakdown.any(item => item.at("exemption_type", default: none) == "tax_exempt")

            #if has_reverse_charge {
              text(size: 9pt, translations.tax_reverse_charge)
              linebreak()
              text(size: 8pt, fill: color.footer_text, translations.at("reverse_charge_notice", default: ""))
            } else if has_tax_exempt {
              text(size: 9pt, translations.vat_exempt_legal)
              linebreak()
              text(size: 8pt, fill: color.footer_text, translations.at("vat_exempt_notice", default: ""))
            } else if tax_breakdown.len() == 0 {
              text(size: 9pt, translations.at("no_tax_applied", default: "No tax applied"))
            } else {
              text(size: 9pt, translations.at("tax_included_text", default: "All prices include tax"))
            }

            #if customer.tax_id != none and organization.tax_id != none {
              v(6pt)
              text(size: 8pt, fill: color.footer_text, translations.at("credit_note_vat_directive_notice", default: ""))
            }
          ]
        )
      ]
    )
  }

  v(16pt)

  // Legal information
  if show_legal_info and organization.footer_legal != none {
    grid(
      columns: (1fr),
      [
        #text(fill: color.heading, weight: "medium", size: 10pt, translations.legal_info)
        #v(4pt)
        #text(size: 8pt, fill: color.footer_text, organization.footer_legal)

        #if organization.legal_number != none {
          v(4pt)
          text(size: 8pt, fill: color.footer_text, [
            #translations.at("company_registration", default: "Registration"): #organization.legal_number
          ])
        }
      ]
    )
  }

  // Footer custom information
  if show_footer_custom_info and organization.footer_info != none {
    v(16pt)
    text(size: 8pt, fill: color.footer_text, organization.footer_info)
  }

  // Exchange rate info
  if organization.exchange_rate != none and organization.accounting_currency_code != none and translations.at("exchange_rate_info", default: none) != none {
    v(8pt)
    text(size: 8pt, fill: color.footer_text, translations.exchange_rate_info)
  }
}
