import { RATE, type RateLine } from './rateLines'

import type { RateCard, RateCardEntry } from '@/rpc/api/ratecards/v1/models_pb'

/** The column of a credit card's single rate: its entries carry no currency. */
export const CREDITS = ''

export interface Scheduled {
  from: string
  /** Unset: the key is removed from that day. */
  rate?: string
}

export interface Cell {
  rate?: string
  label?: string
  scheduled: Scheduled[]
}

export interface Row {
  id: string
  key: string[]
  anyOther: boolean
  /** Added in this draft: its key is still editable. */
  added?: boolean
  cells: Record<string, Cell>
}

/** Staged edits of the table, none sent until saved. */
export interface Draft {
  /** Typed text by row and column; '' clears the cell. */
  edits: Record<string, Record<string, string>>
  removed: string[]
  added: { id: string; key: string[] }[]
}

export const emptyDraft: Draft = { edits: {}, removed: [], added: [] }

const rowId = (key: string[]) => key.join('\u0001')
const isAnyOther = (key: string[]) => key.every(pattern => pattern === '*')

export const isMoney = (card: RateCard) => !card.creditUnitId

/** The rows of a card on `today`: its rates in force, keys only scheduled, and Any other last. */
export function buildRows(
  card: RateCard,
  rates: RateCardEntry[],
  entries: RateCardEntry[],
  today: string
): Row[] {
  const rows = new Map<string, Row>()
  const row = (key: string[], anyOther: boolean) => {
    const id = rowId(key)
    let found = rows.get(id)
    if (!found) {
      found = { id, key, anyOther: anyOther || isAnyOther(key), cells: {} }
      rows.set(id, found)
    }
    return found
  }
  const cell = (r: Row, column: string) => (r.cells[column] ??= { scheduled: [] })

  for (const entry of rates) {
    const c = cell(row(entry.key, entry.anyOther), entry.currency ?? CREDITS)
    c.rate = entry.rate
    c.label = entry.label
  }
  const upcoming = entries
    .filter(entry => entry.effectiveFrom > today)
    .sort((a, b) => a.effectiveFrom.localeCompare(b.effectiveFrom))
  for (const entry of upcoming) {
    // A removal scheduled on a key not in force reads as nothing.
    if (entry.rate === undefined && !rows.has(rowId(entry.key))) continue
    cell(row(entry.key, entry.anyOther), entry.currency ?? CREDITS).scheduled.push({
      from: entry.effectiveFrom,
      rate: entry.rate,
    })
  }

  const anyOtherKey = card.dimensions.map(() => '*')
  row(anyOtherKey, true)
  return [...rows.values()].sort((a, b) =>
    a.anyOther !== b.anyOther
      ? Number(a.anyOther) - Number(b.anyOther)
      : a.key.join(',').localeCompare(b.key.join(','))
  )
}

/** The rate columns: a money card's currencies (the tenant's first), or the credit column. */
export function columnsOf(
  card: RateCard,
  tenantCurrencies: string[],
  rows: Row[],
  draft: Draft
): string[] {
  if (!isMoney(card)) return [CREDITS]
  const columns = [...tenantCurrencies]
  const extra = new Set<string>()
  for (const r of rows) Object.keys(r.cells).forEach(c => extra.add(c))
  for (const edits of Object.values(draft.edits)) Object.keys(edits).forEach(c => extra.add(c))
  return [...columns, ...[...extra].filter(c => c && !columns.includes(c)).sort()]
}

/** The table as shown: the card's rows, then the draft's, Any other kept last. */
export function draftRows(rows: Row[], draft: Draft): Row[] {
  const added: Row[] = draft.added.map(a => ({
    id: a.id,
    key: a.key,
    anyOther: false,
    added: true,
    cells: {},
  }))
  const anyOther = rows.filter(r => r.anyOther)
  return [...rows.filter(r => !r.anyOther), ...added, ...anyOther]
}

const normalize = (rate: string) => {
  const [int, frac = ''] = rate.trim().split('.')
  const trimmedFrac = frac.replace(/0+$/, '')
  const trimmedInt = int.replace(/^0+(?=\d)/, '')
  return trimmedFrac ? `${trimmedInt}.${trimmedFrac}` : trimmedInt
}

export const sameRate = (a: string | undefined, b: string | undefined) =>
  a === undefined || b === undefined ? a === b : normalize(a) === normalize(b)

const hasRate = (c: Cell | undefined) =>
  !!c && (c.rate !== undefined || c.scheduled.some(s => s.rate !== undefined))

export interface Change {
  rates: { key: string[]; currency?: string; rate: string; label?: string }[]
  remove: { key: string[]; currency?: string }[]
  /** Why the draft can't be saved, by row id (and column for a cell). */
  errors: Record<string, string>
  /** Only first rates of key and currency: the batch may start on a past day. */
  firstRatesOnly: boolean
}

/** An added row's key: a cell left empty matches any value. */
const addedKey = (key: string[]) => key.map(pattern => pattern.trim() || '*')

/**
 * The text a rate cell takes from a keystroke, or undefined to refuse it: digits and one
 * decimal point (a comma reads as one), so "ssss" never reaches the draft.
 */
export function acceptRateInput(text: string): string | undefined {
  const typed = text.replace(/\s/g, '').replace(',', '.')
  return /^\d*\.?\d*$/.test(typed) ? typed : undefined
}

/** A typed rate as sent: "2." is 2 and ".5" is 0.5, mid-typing states of a number. */
const cleanRate = (text: string) => {
  const typed = text.trim().replace(/\.$/, '')
  return typed.startsWith('.') ? `0${typed}` : typed
}

/** What saving the draft sends, or why it can't be. */
export function changeOf(rows: Row[], draft: Draft): Change {
  const change: Change = { rates: [], remove: [], errors: {}, firstRatesOnly: true }
  const currencyOf = (column: string) => (column === CREDITS ? undefined : column)
  const removeCell = (key: string[], column: string) => {
    change.remove.push({ key, currency: currencyOf(column) })
    change.firstRatesOnly = false
  }

  for (const r of rows) {
    if (draft.removed.includes(r.id)) {
      for (const [column, c] of Object.entries(r.cells)) if (hasRate(c)) removeCell(r.key, column)
      continue
    }
    for (const [column, text] of Object.entries(draft.edits[r.id] ?? {})) {
      const c = r.cells[column]
      const rate = cleanRate(text)
      if (!rate) {
        if (hasRate(c)) removeCell(r.key, column)
      } else if (!RATE.test(rate)) {
        change.errors[`${r.id}/${column}`] = 'Not a number'
      } else if (!sameRate(rate, c?.rate)) {
        change.rates.push({ key: r.key, currency: currencyOf(column), rate, label: c?.label })
        if (hasRate(c)) change.firstRatesOnly = false
      }
    }
  }

  const taken = new Set(rows.filter(r => !draft.removed.includes(r.id)).map(r => rowId(r.key)))
  for (const a of draft.added) {
    const key = addedKey(a.key)
    const edits = Object.entries(draft.edits[a.id] ?? {}).filter(([, text]) => cleanRate(text))
    if (!edits.length && a.key.every(pattern => !pattern.trim())) continue
    if (isAnyOther(key)) {
      change.errors[a.id] = 'A row matching anything is Anything else, below'
      continue
    }
    if (taken.has(rowId(key))) {
      change.errors[a.id] = 'This row already exists'
      continue
    }
    taken.add(rowId(key))
    for (const [column, text] of edits) {
      const rate = cleanRate(text)
      if (!RATE.test(rate)) change.errors[`${a.id}/${column}`] = 'Not a number'
      else change.rates.push({ key, currency: currencyOf(column), rate })
    }
  }
  return change
}

let addedRows = 0
export const newRowId = () => `new-${++addedRows}`

/** The draft with imported lines staged: a known key's cell is edited, a new key is added. */
export function importLines(rows: Row[], draft: Draft, lines: RateLine[]): Draft {
  const next: Draft = {
    edits: { ...draft.edits },
    removed: [...draft.removed],
    added: [...draft.added],
  }
  for (const line of lines) {
    const id = rowId(line.key)
    let target =
      rows.find(r => r.id === id)?.id ??
      next.added.find(a => rowId(addedKey(a.key)) === id)?.id
    if (!target) {
      target = newRowId()
      next.added.push({ id: target, key: line.key })
    }
    next.removed = next.removed.filter(r => r !== target)
    next.edits[target] = { ...next.edits[target], [line.currency ?? CREDITS]: line.rate }
  }
  return next
}

/** The rates in force, a line per key and currency, as `parseRateLines` reads them back. */
export function exportLines(card: RateCard, rows: Row[]): string[][] {
  const lines: string[][] = []
  for (const r of rows) {
    for (const [column, c] of Object.entries(r.cells)) {
      if (c.rate === undefined) continue
      lines.push(isMoney(card) ? [...r.key, column, c.rate] : [...r.key, c.rate])
    }
  }
  return lines
}

const SHORT_SIZES: Record<string, string> = { '1': '1', '1000': '1K', '1000000': '1M' }

/** "per 1M tokens", "per token", "per 1K". */
export function unitSizeLabel(
  unitSize: string,
  unit?: { singular: string; plural: string }
): string {
  const size = normalize(unitSize || '1')
  const short = SHORT_SIZES[size] ?? Number(size).toLocaleString()
  if (!unit?.singular) return `per ${short}`
  return size === '1' ? `per ${unit.singular}` : `per ${short} ${unit.plural || unit.singular}`
}

export const UNIT_SIZES = [
  { value: '1', label: 'per 1' },
  { value: '1000', label: 'per 1K' },
  { value: '1000000', label: 'per 1M' },
]

const ANY_UNIT = { singular: 'unit', plural: 'units' }

/** The metric's unit name, "unit" when it has none. */
export const unitOf = (unit?: { singular: string; plural: string }) =>
  unit?.singular ? unit : ANY_UNIT

/** A dimension's event property key in words: "token_type" and "tokenType" read "Token type". */
export function dimensionName(key: string): string {
  const words = key
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/[_.\-\s]+/g, ' ')
    .trim()
    .toLowerCase()
  return words ? words[0].toUpperCase() + words.slice(1) : key
}

/** "Model", "Model and Token type", "Model, Region and Token type". */
export function listNames(names: string[]): string {
  if (names.length < 2) return names.join('')
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`
}

/** "What each token costs, per 1M tokens, by Model and Token type." */
export function rateCardHeading(
  card: Pick<RateCard, 'dimensions' | 'unitSize'>,
  unit?: { singular: string; plural: string },
  creditUnitName?: string
): string {
  const u = unitOf(unit)
  const per = normalize(card.unitSize || '1') === '1' ? '' : `, ${unitSizeLabel(card.unitSize, u)}`
  const credits = creditUnitName ? ` in ${creditUnitName}` : ''
  const by = listNames(card.dimensions.map(dimensionName))
  return `What each ${u.singular} costs${credits}${per}, by ${by}.`
}

/** "3 rates in EUR, USD · per 1M tokens": a rate per key, whatever its currencies. */
export function rateCardSummary(
  card: Pick<RateCard, 'creditUnitId' | 'unitSize'>,
  rates: Pick<RateCardEntry, 'key' | 'currency' | 'rate'>[],
  unit?: { singular: string; plural: string },
  creditUnitName?: string
): string {
  const per = unitSizeLabel(card.unitSize, unitOf(unit))
  const rated = rates.filter(r => r.rate !== undefined)
  const keys = new Set(rated.map(r => rowId(r.key))).size
  if (!keys) return `No rates yet · ${per}`
  const currencies = [...new Set(rated.flatMap(r => (r.currency ? [r.currency] : [])))].sort()
  const what = card.creditUnitId ? creditUnitName || 'credits' : currencies.join(', ')
  return `${keys} ${keys === 1 ? 'rate' : 'rates'} in ${what} · ${per}`
}

/** "€" for EUR, "$" for USD; the code itself when the platform has no symbol for it. */
export function currencySymbol(code: string): string {
  try {
    const parts = new Intl.NumberFormat('en', {
      style: 'currency',
      currency: code,
      currencyDisplay: 'narrowSymbol',
    }).formatToParts(0)
    return parts.find(part => part.type === 'currency')?.value ?? code
  } catch {
    return code
  }
}
