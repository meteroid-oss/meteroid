import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { InvoicePaymentStatus, InvoiceSchema, InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'

import { daysPastDue, invoiceRowState, openLabel, overdueLabel } from './invoiceState'

const utc = (iso: string) => new Date(`${iso}Z`)
const asOf = utc('2026-10-09T16:20:00')
const year = '(, 2026)?'

describe('daysPastDue', () => {
  it('counts whole UTC days, 0 on the due day', () => {
    expect(daysPastDue(utc('2026-10-09T00:00:00'), utc('2026-10-09T23:00:00'))).toBe(0)
    expect(daysPastDue(utc('2026-10-08T00:00:00'), utc('2026-10-09T00:30:00'))).toBe(1)
  })
})

describe('invoiceRowState', () => {
  const invoice = (paymentStatus: InvoicePaymentStatus, dueAt?: string) =>
    create(InvoiceSchema, {
      status: InvoiceStatus.FINALIZED,
      paymentStatus,
      amountDue: 50000n,
      currency: 'USD',
      dueAt,
    })

  it('reads an unpaid invoice past due as Overdue with whole days (Kestrel, 10 days)', () => {
    expect(
      invoiceRowState(invoice(InvoicePaymentStatus.UNPAID, '2026-09-29T00:00:00'), asOf)
    ).toEqual({
      kind: 'overdue',
      days: 10,
    })
  })

  it('reads an invoice due today as Open, and Overdue · 1 day the next day', () => {
    const today = asOf.toISOString().slice(0, 10)
    expect(
      invoiceRowState(invoice(InvoicePaymentStatus.UNPAID, `${today}T00:00:00`), asOf).kind
    ).toBe('open')
    const yesterday = new Date(asOf.getTime() - 86_400_000).toISOString().slice(0, 10)
    expect(
      invoiceRowState(invoice(InvoicePaymentStatus.UNPAID, `${yesterday}T00:00:00`), asOf)
    ).toEqual({ kind: 'overdue', days: 1 })
  })

  it('reads a net-30 invoice not yet due as Open (Meridian)', () => {
    const state = invoiceRowState(invoice(InvoicePaymentStatus.UNPAID, '2026-10-31T00:00:00'), asOf)
    expect(state.kind).toBe('open')
    expect(state.kind === 'open' && state.due).toMatch(new RegExp(`^Oct 31${year}$`))
  })

  it('leaves paid and in-flight invoices to their own badge', () => {
    expect(
      invoiceRowState(invoice(InvoicePaymentStatus.PAID, '2026-09-01T00:00:00'), asOf).kind
    ).toBe('settled')
    expect(
      invoiceRowState(invoice(InvoicePaymentStatus.PROCESSING, '2026-09-01T00:00:00'), asOf).kind
    ).toBe('settled')
  })
})

describe('overdueLabel', () => {
  it('counts the days late, singular for one', () => {
    expect(overdueLabel(1)).toBe('Overdue · 1 day')
    expect(overdueLabel(12)).toBe('Overdue · 12 days')
  })
})

describe('openLabel', () => {
  it('names the due date when there is one', () => {
    expect(openLabel('Oct 31')).toBe('Open · due Oct 31')
    expect(openLabel()).toBe('Open')
  })
})
