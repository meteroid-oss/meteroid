// Core templates for the invoice system

// Footer block for invoices
#let footer-block(organization, translations) = {
  set text(size: 9pt)

  grid(
    columns: (1fr, 1fr),
    rows: (auto, auto, auto),
    gutter: 8pt,

    grid.hline(y: 0, stroke: 0.5pt + rgb(220, 220, 220), position: top),

    [#organization.name],

    if organization.tax_id != none [
      #grid.cell("Tax ID: " + organization.tax_id, align: right)
    ] else [],

    if organization.address.line1 != none [
      #organization.address.line1
    ] else [],

    if organization.legal_number != none [
      #grid.cell("Reg. no: " + organization.legal_number, align: right)
    ] else [],

    if organization.email != none [
      #organization.email
    ] else [],

    []
  )
}

// Main letter layout template
#let letter-simple(
  sender: (
    name: none,
    company: none,
    address: none,
    extra: none,
  ),

  recipient: none,
  logo: none,

  footer: none,

  folding-marks: false,
  hole-mark: false,

  reference-signs: none,

  date: none,
  subject: none,

  page-numbering: (current-page, page-count) => {
    "Page " + str(current-page) + " of " + str(page-count)
  },

  margin: (
    left:   25mm,
    right:  20mm,
    top:    20mm,
    bottom: 20mm,
  ),

  body,
) = {
  // Set up the page
  set document(
    title: subject,
    author: sender.name,
  )

  set page(
    paper: "a4",
    margin: margin,

    header: {
      grid(
        columns: (2fr, 1fr),

        if sender.name != none /*or sender.company != none*/ or sender.address != none {
          align(left + top, {
            //if sender.company != none {
            //  text(weight: "bold", sender.company)
            //  linebreak()
            //}
            if sender.name != none {
              text(weight: "bold", sender.name)
              linebreak()
            }
            if sender.address != none {
              sender.address
              linebreak()
            }
            if sender.extra != none {
              v(0.5em)
              sender.extra
            }
          })
        } else { [] },

        align(right + top, {
          if logo != none {
            logo
          }
        })
      )
    },

    footer: {
      if footer != none {
        footer
      }

      if page-numbering != none and counter(page).final() > 1 {
        align(center, text(size: 9pt, page-numbering(
          counter(page).get(),
          counter(page).final()
        )))
      }
    },

    // Background elements for folding marks and hole mark
    background: {
      if folding-marks {
        place(top + left, dx: 5mm, dy: 105mm, line(
          length: 2.5mm,
          stroke: 0.25pt + black
        ))
        place(top + left, dx: 5mm, dy: 210mm, line(
          length: 2.5mm,
          stroke: 0.25pt + black
        ))
      }

      if hole-mark {
        place(left + top, dx: 5mm, dy: 148.5mm, line(
          length: 4mm,
          stroke: 0.25pt + black
        ))
      }
    }
  )

  // Recipient address
  grid(
    columns: 1,
    rows: auto,

    box(
      width: 85mm,
      height: 45mm,
      inset: (top: 20mm, rest: 0mm),
      {
        if recipient != none {
          recipient
        }
      }
    ),
  )

// Reference information
  if reference-signs != none and reference-signs.len() > 0 {
    v(5mm)
    grid(
      columns: (1fr, 1fr, 1fr),
      rows: auto,
      gutter: 5mm,

      ..reference-signs.map(sign => {
        let (key, value) = sign

        [
          #text(size: 10pt, key)
          #linebreak()
          #text(size: 10pt, weight: "regular", value)
        ]
      })
    )
    v(5mm)
  }

  // Subject line
  if subject != none {
    v(5mm)
    align(left, heading(
      level: 1,
      subject
    ))
    v(5mm)
  }

  // Date
  if date != none {
    align(right, date)
    v(5mm)
  }

  // Main body content
  body
}
