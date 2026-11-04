import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { parseUtc } from '@/lib/utils/date'
import { InvoicePaymentStatus, InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'

import type { Invoice } from '@/rpc/api/invoices/v1/models_pb'

const DAY_MS = 24 * 60 * 60 * 1000

export type InvoiceRowState =
  { kind: 'overdue'; days: number } | { kind: 'open'; due?: string } | { kind: 'settled' }

const utcDay = (date: Date) =>
  Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate())

/** Whole UTC days an invoice due on `due` is late by `now`: 0 on its due day, as the server counts. */
export const daysPastDue = (due: Date, now: Date) =>
  Math.max(0, Math.round((utcDay(now) - utcDay(due)) / DAY_MS))

/** An issued invoice still owed reads as Overdue from the day after its due date, else Open. */
export const invoiceRowState = (invoice: Invoice, now: Date): InvoiceRowState => {
  const owed =
    invoice.status === InvoiceStatus.FINALIZED &&
    !invoice.consolidatedIntoInvoiceId &&
    invoice.amountDue > 0n &&
    (invoice.paymentStatus === InvoicePaymentStatus.UNPAID ||
      invoice.paymentStatus === InvoicePaymentStatus.PARTIALLY_PAID ||
      invoice.paymentStatus === InvoicePaymentStatus.ERRORED)
  if (!owed) return { kind: 'settled' }
  if (!invoice.dueAt) return { kind: 'open' }
  const due = parseUtc(invoice.dueAt)
  const days = daysPastDue(due, now)
  if (days > 0) return { kind: 'overdue', days }
  return { kind: 'open', due: formatShortDay(due.toISOString()) }
}

export const overdueLabel = (days: number) => `Overdue · ${days} ${days === 1 ? 'day' : 'days'}`

export const openLabel = (due?: string) => (due ? `Open · due ${due}` : 'Open')
