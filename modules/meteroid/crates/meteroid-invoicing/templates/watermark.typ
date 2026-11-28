// A small tag in the top margin of every page a non-production tenant issued.
#let test_watermark(on) = if on {
  place(top + right, dx: -36pt, dy: 18pt, box(
    fill: rgb("#fef3c7"),
    stroke: 0.5pt + rgb("#fcd34d"),
    radius: 3pt,
    inset: (x: 6pt, y: 3pt),
    text(font: "inter", size: 7pt, weight: "medium", fill: rgb("#92400e"), "Test mode – Not a real document"),
  ))
}
