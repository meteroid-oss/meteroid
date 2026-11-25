/**
 * A rate card price's markup is entered in percent ("-20": 20% off) and stored as the price's
 * multiplier (0.8). Decimal strings throughout, so 7% is 1.07 and not 1.0699999.
 */

const DECIMAL = /^-?\d+(\.\d+)?$/

const scaled = (value: string): { digits: bigint; scale: number } | undefined => {
  const text = value.trim()
  if (!DECIMAL.test(text)) return undefined
  const [int, frac = ''] = text.replace('-', '').split('.')
  const digits = BigInt(int + frac)
  return { digits: text.startsWith('-') ? -digits : digits, scale: frac.length }
}

const format = (digits: bigint, scale: number): string => {
  const negative = digits < 0n
  const text = (negative ? -digits : digits).toString().padStart(scale + 1, '0')
  const int = text.slice(0, text.length - scale)
  const frac = text.slice(text.length - scale).replace(/0+$/, '')
  const body = frac ? `${int}.${frac}` : int
  return negative && body !== '0' ? `-${body}` : body
}

/** The multiplier of a markup in percent; undefined when the markup is empty or zero. */
export function markupToMultiplier(markup: string | undefined): string | undefined {
  const pct = markup ? scaled(markup) : undefined
  if (!pct || pct.digits === 0n) return undefined
  return format(100n * 10n ** BigInt(pct.scale) + pct.digits, pct.scale + 2)
}

/** The markup in percent of a multiplier; '' when unset or 1 (the card's rates as they are). */
export function multiplierToMarkup(multiplier: string | undefined): string {
  const m = multiplier ? scaled(multiplier) : undefined
  if (!m) return ''
  const pct = (m.digits - 10n ** BigInt(m.scale)) * 100n
  return pct === 0n ? '' : format(pct, m.scale)
}

/** "card rates", "card rates +20%", "card rates −20%", from the markup in percent. */
export function cardRatesLabel(markup: string | undefined): string {
  const pct = markup ? scaled(markup) : undefined
  if (!pct || pct.digits === 0n) return 'card rates'
  const body = format(pct.digits < 0n ? -pct.digits : pct.digits, pct.scale)
  return `card rates ${pct.digits < 0n ? '−' : '+'}${body}%`
}

/** The same label from a stored multiplier. */
export const cardRatesLabelOfMultiplier = (multiplier: string | undefined) =>
  cardRatesLabel(multiplierToMarkup(multiplier))
