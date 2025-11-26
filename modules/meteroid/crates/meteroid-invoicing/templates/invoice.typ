#import sys: inputs

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
    button: rgb(50, 125, 230),
    white: rgb(255, 255, 255),
    subline_text: rgb(55, 55, 55),
    // Status colors
    paid: rgb(76, 175, 80),
    partially_paid: rgb(255, 152, 0),
    unpaid: rgb(244, 67, 54),
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
            ]) • #number • #format_amount(total_amount) #translations.due_label #due_date
          ])
        } else {
          text(font: inter, size: 8pt, fill: color.footer_text, [
            #number • #format_amount(total_amount) #translations.due_label #due_date
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
  set document(title: translations.invoice_title + " " + number)

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

    // Logo aligned right - reduced size
    if organization.logo_src != none {
      align(right, image(organization.logo_src, height: 30pt, width: 150pt, fit: "contain"))
    } else {
      align(right, image("logo.png", width: 30pt))
    }
  )

  v(16pt)

  grid(
    columns: (120pt, auto),
    rows: (auto, auto, auto, auto),
    row-gutter: 6pt,

    [#text(fill: color.accent, weight: "medium", translations.invoice_number)],
    [#text(weight: "medium", number)],

    [#text(fill: color.accent, weight: "medium", translations.issue_date)],
    [#text(weight: "medium", issue_date)],

    [#text(fill: color.accent, weight: "medium", translations.due_date)],
    [#text(weight: "medium", due_date)],

    if purchase_order != none [
      #text(fill: color.accent, weight: "medium", translations.purchase_order)
    ] else [],

   if purchase_order != none [
      #text(weight: "medium", purchase_order)
   ] else [],

    if organization.tax_id != none [
      #text(fill: color.accent, weight: "medium", translations.vat_id)
    ] else [],

    if organization.tax_id != none [
      #text(weight: "medium", organization.tax_id)
    ] else [],
  )

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
        #text(size: 16pt, weight: "bold", fill: color.heading, [
          #format_amount(total_amount)
        ])
         #text(size: 12pt, weight: "bold", fill: color.heading, [
          #translations.due_label #due_date
        ])


        #if memo != none {
          text(fill: color.accent, [
            #memo
          ])
        }

        #v(4pt)

        // Add payment button with url link if provided
        #if pay_online_url != none {
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

  // Line items with compact styling and improved sublines
  for (index, item) in lines.enumerate() {
    grid(
      columns: (4fr, 1fr, 1fr, 0.8fr, 1.2fr),
      column-gutter: 6pt,
      row-gutter: 0pt, // Remove row gap

      [
        #text(weight: "medium", fill: color.heading, item.name)
        #if item.description != none [
          #text(size: 9pt, fill: color.accent, item.description)
        ]
        #linebreak() // Keep dates on new line as requested
        #text(size: 8pt, fill: color.date_text, item.start_date + " → " + item.end_date)
      ],

      align(center, text(weight: "regular", if item.quantity != none { str(item.quantity) } else { "" })),

      align(right, text(weight: "regular", if item.unit_price != none { format_amount(item.unit_price) } else { "" })),

      align(right, text(weight: "regular", if item.vat_rate != none { str(item.vat_rate) + "%" } else { "" })),

      align(right, text(weight: "regular", format_amount(item.subtotal))),
    )

    // Add sublines if they exist
    if item.sub_lines != none and item.sub_lines.len() > 0 {

      // Container for all sublines with background color
      block(
        width: 100%,
        radius: 3pt,
        inset: (x: 3pt, y: 2pt),
        [
          // Iterate through sublines
          #for (sub_index, sub_item) in item.sub_lines.enumerate() {
            grid(
              columns: (4fr, 1fr, 1fr, 0.8fr, 1.2fr),
              column-gutter: 6pt,
              row-gutter: 0pt,

              // Subline with indent and icon
              [
                #box(width: 12pt, [])
                #text(size: 8.5pt,   fill: color.accent, [
                  #sub_item.name
                ])
              ],

              align(center, text(size: 8.5pt, fill: color.accent, if sub_item.quantity != none { str(sub_item.quantity) } else { "" })),

              align(right, text(size: 8.5pt, fill: color.accent, if sub_item.unit_price != none { format_amount(sub_item.unit_price) } else { "" })),

              [], // Empty tax rate column for sublines

              align(right, text(size: 8.5pt, fill: color.accent, format_amount(sub_item.total))),
            )

          }
        ]
      )
    }

    // Add minimal spacing between items
    if index < lines.len() - 1 {
      v(3pt)
      line(length: 100%, stroke: 0.75pt + color.light_border)
      v(3pt)
    }
  }

  v(16pt)

  // Summary section aligned right with payment status on the left
  grid(
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

        // Show discount if applicable
        ..if discount > 0 {
          (
            text(fill: color.accent, translations.discount),
            align(right, text(weight: "regular", "-" + format_amount(discount))),
          )
        },

        ..for coupon in coupons {
          (
            text(fill: color.accent, coupon.name),
            align(right, text(weight: "regular", "-" + format_amount(coupon.total))),
          )
        },


        ..if tax_breakdown.len() > 0 and tax_amount > 0 {
          // Show tax breakdown for any non-zero tax
          for tax_item in tax_breakdown {
            (
              text(fill: color.accent, tax_item.name + " " + str(tax_item.rate) + "%"),
              align(right, text(weight: "regular", format_amount(tax_item.amount))),
            )
          }
        },

        text(weight: "medium", size: 12pt, fill: color.heading, translations.total_due),
        align(right, text(weight: "medium", size: 12pt, fill: color.heading, format_amount(total_amount))),
      )
    ])
  )

  v(30pt)

  // Add payment information section if provided and enabled
  if show_payment_info and payment_info != none {
    block(
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

            // EU compliance notice for international transactions
            #if customer.tax_id != none and organization.tax_id != none {
              v(6pt)
              text(size: 8pt, fill: color.footer_text, translations.at("eu_vat_directive_notice", default: ""))
            }
          ] else []
        )
      ]
    )
  }

  v(16pt)

  // Legal information - only if enabled
  if show_legal_info and organization.footer_legal != none {
    grid(
      columns: (1fr),
      [
        #text(fill: color.heading, weight: "medium", size: 10pt, translations.legal_info)
        #v(4pt)
        #text(size: 8pt, fill: color.footer_text, organization.footer_legal)

        // Add company registration info if available
        #if organization.legal_number != none {
          v(4pt)
          text(size: 8pt, fill: color.footer_text, [
            #translations.at("company_registration", default: "Registration"): #organization.legal_number
          ])
        }
      ]
    )
  }

  // Footer custom information - only if enabled
  if show_footer_custom_info and organization.footer_info != none {
    v(16pt)
    text(size: 8pt, fill: color.footer_text, organization.footer_info)
  }

  // Add exchange rate info if available
  if organization.exchange_rate != none and organization.accounting_currency_code != none and translations.at("exchange_rate_info", default: none) != none {
    v(8pt)
    text(size: 8pt, fill: color.footer_text, translations.exchange_rate_info)
  }
}

