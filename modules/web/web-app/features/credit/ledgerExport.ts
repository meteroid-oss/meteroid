import { timestampDate } from '@bufbuild/protobuf/wkt'
import Decimal from 'decimal.js'

import { currencyPrecision } from '@/lib/data/currency-precision'
import { downloadCsv } from '@/lib/utils/csv'

import type { Timestamp } from '@bufbuild/protobuf/wkt'

export const LEDGER_PAGE_SIZE = 10

/** Where the server streams a customer's credit statement as CSV, for a signed export token. */
export const customerLedgerCsvUrl = (apiUri: string, customerId: string, token: string) =>
  `${apiUri}/files/v1/credit-ledger/${encodeURIComponent(customerId)}?token=${encodeURIComponent(token)}`
// The most the ledger RPCs serve per page.
const EXPORT_PAGE_SIZE = 100

type Page<E> = { entries: E[]; totalPages: number }

/**
 * Every entry of a newest-first, offset-paginated ledger. Entries recorded while exporting shift
 * older ones to later pages, so repeats are dropped by id; the ledger is append-only, so none is
 * missed.
 */
export const collectLedger = async <E extends { id: string }>(
  fetchPage: (page: number, perPage: number) => Promise<Page<E>>
): Promise<E[]> => {
  const seen = new Set<string>()
  const entries: E[] = []
  for (let page = 0, totalPages = 1; page < totalPages; page++) {
    const result = await fetchPage(page, EXPORT_PAGE_SIZE)
    totalPages = result.totalPages
    if (result.entries.length === 0) break
    for (const entry of result.entries) {
      if (seen.has(entry.id)) continue
      seen.add(entry.id)
      entries.push(entry)
    }
  }
  return entries
}

// Text a spreadsheet would run as a formula is kept as text.
const textCell = (value: string | undefined) =>
  value && /^[=+\-@\t\r]/.test(value) ? `'${value}` : (value ?? '')

export type LedgerCsvRow = {
  date?: Timestamp
  movement: string
  description: string
  reference?: string
  amount: bigint
  balanceAfter: bigint
  invoiceId?: string
  creditNoteId?: string
}

// A credit unit wallet passes its unit's precision: the portal has no tenant unit catalog.
export const downloadLedgerCsv = (
  filename: string,
  currency: string,
  rows: LedgerCsvRow[],
  precision: number = currencyPrecision(currency)
) =>
  downloadCsv(
    filename,
    [
      'Date (UTC)',
      'Movement',
      'Description',
      'Reference',
      'Amount',
      'Balance after',
      'Currency',
      'Invoice',
      'Credit note',
    ],
    rows.map(row => [
      row.date ? timestampDate(row.date).toISOString() : '',
      row.movement,
      textCell(row.description),
      textCell(row.reference),
      signedDecimal(row.amount, precision),
      signedDecimal(row.balanceAfter, precision),
      currency,
      row.invoiceId ?? '',
      row.creditNoteId ?? '',
    ])
  )

const signedDecimal = (minor: bigint, precision: number) =>
  new Decimal(minor.toString()).div(new Decimal(10).pow(precision)).toFixed()
