export interface RateLine {
  key: string[]
  /** Unset on a credit card's lines. */
  currency?: string
  rate: string
}

export const RATE = /^\d+(\.\d+)?$/

const unquote = (cell: string) => cell.trim().replace(/^"(.*)"$/, '$1').replace(/""/g, '"')

/**
 * Lines of `pattern, …, currency, rate` (`pattern, …, rate` on a credit card), a pattern per
 * dimension, comma or tab separated: a sheet export pastes as is. A header line is skipped.
 */
export function parseRateLines(
  text: string,
  dimensions: number,
  money: boolean
): { lines: RateLine[] } | { error: string } {
  const lines: RateLine[] = []
  const width = dimensions + (money ? 2 : 1)
  const rows = text.split(/\r?\n/).map(line => line.trim())
  for (const [index, line] of rows.entries()) {
    if (!line) continue
    const cells = line.split(line.includes('\t') ? '\t' : ',').map(unquote)
    const rate = cells[cells.length - 1]
    if (lines.length === 0 && rate?.toLowerCase() === 'rate') continue
    const at = `Line ${index + 1}`
    if (cells.length !== width) {
      return {
        error: `${at}: ${dimensions} patterns${money ? ', a currency' : ''} and a rate expected`,
      }
    }
    const key = cells.slice(0, dimensions)
    if (key.some(pattern => !pattern)) return { error: `${at}: a pattern is empty (* for any)` }
    if (!RATE.test(rate)) return { error: `${at}: "${rate}" is not a rate` }
    if (money) {
      const currency = cells[dimensions].toUpperCase()
      if (!/^[A-Z]{3}$/.test(currency)) {
        return { error: `${at}: "${cells[dimensions]}" is not a currency code` }
      }
      lines.push({ key, currency, rate })
    } else {
      lines.push({ key, rate })
    }
  }
  return { lines }
}
