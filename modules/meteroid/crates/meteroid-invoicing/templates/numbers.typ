// Numbers in the document's language. Mirrors `format::grouped_number` and `format::amount`.

#let marks(lang) = if lang == "en-US" {
  (",", ".")
} else if lang == "fr-FR" or lang == "pt-PT" {
  ("\u{00A0}", ",")
} else {
  (".", ",")
}

// `value` is printed at its own scale: `str` of a decimal keeps it, and writes `−` (U+2212).
#let format_number(lang, value) = {
  let (group_mark, decimal_mark) = marks(lang)
  let digits = str(value)
  let sign = if digits.starts-with("\u{2212}") { "\u{2212}" } else { "" }
  let parts = digits.slice(sign.len()).split(".")
  let whole = parts.at(0).clusters()
  let grouped = whole.enumerate().map(((i, digit)) => {
    if i > 0 and calc.rem(whole.len() - i, 3) == 0 { group_mark + digit } else { digit }
  }).join()
  sign + grouped + if parts.len() > 1 { decimal_mark + parts.at(1) } else { "" }
}

#let format_amount(lang, currency_symbol, amount) = if lang.starts-with("fr") {
  format_number(lang, amount) + "\u{00A0}" + currency_symbol
} else {
  currency_symbol + "\u{00A0}" + format_number(lang, amount)
}

#let format_percent(lang, rate) = format_number(lang, rate) + "%"
