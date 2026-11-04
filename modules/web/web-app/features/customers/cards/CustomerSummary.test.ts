import { describe, expect, it } from 'vitest'

import { nextPaymentDate } from './CustomerSummary'

describe('nextPaymentDate', () => {
  const invoice = { chargeDate: '2026-11-19', dueDate: '2026-11-19', invoiceDate: '2026-11-18' }

  it('is the charge date of an invoice charged automatically, never its invoice date', () => {
    expect(nextPaymentDate(invoice, true)).toBe('2026-11-19')
    expect(nextPaymentDate({ ...invoice, chargeDate: '' }, true)).toBe('2026-11-18')
  })

  it('is the due date of an invoice paid by the customer', () => {
    expect(nextPaymentDate({ ...invoice, dueDate: '2026-12-18' }, false)).toBe('2026-12-18')
    expect(nextPaymentDate({ ...invoice, dueDate: undefined }, false)).toBe('2026-11-18')
  })
})
