import { create } from '@bufbuild/protobuf'
import { timestampFromDate } from '@bufbuild/protobuf/wkt'
import { describe, expect, it } from 'vitest'

import { InvoicePaymentStatus, InvoiceSchema, InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'
import {
  SubscriptionFeeBillingPeriod,
  SubscriptionFeeSchema,
  SubscriptionStatus,
} from '@/rpc/api/subscriptions/v1/models_pb'
import {
  OverdueInvoiceSchema,
  PendingInvoiceCreditSchema,
  SubscriptionReceivablesSchema,
  UpcomingInvoiceChange,
} from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import {
  CreditGrantSource,
  CreditGrantStatus,
  CreditGrantType,
  WalletSchema,
  WalletStatus,
} from '@/rpc/api/wallets/v1/models_pb'

import {
  averageUsage,
  creditNote,
  formatBalance,
  formatBareAmount,
  idleWalletDetail,
  invoiceAfterCredit,
  issueDay,
  nextInvoiceAmount,
  nextInvoiceTitle,
  isIdleWallet,
  ledgerStatus,
  limitRefused,
  overdueDetails,
  outstandingFact,
  packRollover,
  packSplit,
  primaryAction,
  sinceLabel,
  unitShortName,
  usableCredit,
  walletlessCredit,
} from './subscriptionOverview'

import type { MessageInitShape } from '@bufbuild/protobuf'

const utc = (iso: string) => new Date(`${iso}Z`)
const asOf = utc('2026-10-09T16:20:00')
const year = '(, 2026)?'

type WalletInit = MessageInitShape<typeof WalletSchema>
const wallet = (init: WalletInit) =>
  create(WalletSchema, {
    id: 'w1',
    currency: 'USD',
    status: WalletStatus.HEALTHY,
    ...init,
  })
const live = (availableAfterUsage: bigint, pendingUsage: bigint, uncoveredUsage = 0n) => ({
  availableAfterUsage,
  pendingUsage,
  uncoveredUsage,
  asOf: timestampFromDate(asOf),
  stale: false,
})
const grant = (init: {
  remaining: bigint
  expiresAt?: string
  type?: CreditGrantType
  status?: CreditGrantStatus
  reason?: string
  effectiveAt?: string
}) => ({
  id: `g-${init.remaining}`,
  amount: init.remaining,
  remaining: init.remaining,
  type: init.type ?? CreditGrantType.PAID,
  status: init.status ?? CreditGrantStatus.ACTIVE,
  source: CreditGrantSource.PURCHASE,
  reason: init.reason ?? '',
  expiresAt: init.expiresAt ? timestampFromDate(utc(init.expiresAt)) : undefined,
  effectiveAt: init.effectiveAt ? timestampFromDate(utc(init.effectiveAt)) : undefined,
})

// Tessera Health on Scale: $550 on account − $150 unbilled = $400 balance.
const tessera = wallet({
  available: 55000n,
  live: live(40000n, 15000n),
  grants: [grant({ remaining: 55000n, expiresAt: '2026-10-29T00:00:00' })],
})
const scalePeriod = { periodStart: '2026-09-29T00:00:00Z', periodEnd: '2026-10-29T00:00:00Z' }

describe('ledger credit of a subscription', () => {
  const shared = wallet({
    available: 9000n,
    status: WalletStatus.LOW,
    groups: [
      { label: 'Account balance', available: 4000n },
      { label: 'Included with Scale', subscriptionId: 'scale', available: 3000n },
      { label: 'Included with Studio', subscriptionId: 'studio', available: 2000n },
    ],
  })

  it("counts its own and the customer-wide credit, never another subscription's", () => {
    expect(usableCredit(shared, 'scale')).toBe(7000n)
    expect(usableCredit(shared, 'payg')).toBe(4000n)
  })

  it("keeps a low status only when the wallet holds no other subscription's credit", () => {
    expect(ledgerStatus(shared, 'scale')).toBe(WalletStatus.UNSPECIFIED)
    expect(ledgerStatus(wallet({ available: 5000n, status: WalletStatus.LOW }), 'scale')).toBe(
      WalletStatus.LOW
    )
    const blocked = wallet({ available: 0n, status: WalletStatus.USAGE_BLOCKED })
    expect(ledgerStatus(blocked, 'scale')).toBe(WalletStatus.UNSPECIFIED)
  })
})

describe('credit row formatting', () => {
  const halyard = {
    id: 'u1',
    code: 'HAL',
    name: 'Halyard credit',
    pluralName: 'Halyard credits',
    precision: 0,
  }

  it('names a credit unit once: short form on the balance, bare elsewhere', () => {
    expect(unitShortName(halyard)).toBe('credits')
    expect(unitShortName({ pluralName: 'Tokens' })).toBe('Tokens')
    const lumen = wallet({ currency: 'HAL', creditUnit: halyard })
    expect(formatBalance(3720n, lumen)).toBe('3,720 credits')
    expect(formatBareAmount(7620n, lumen)).toBe('7,620')
  })

  it('keeps currency arithmetic bare and consistent: on account − unbilled = balance', () => {
    expect(formatBareAmount(55000n, { currency: 'USD' })).toBe('550.00')
    expect(formatBareAmount(15000n, { currency: 'USD' })).toBe('150.00')
    expect(formatBalance(55000n - 15000n, { currency: 'USD' })).toBe('$400.00')
  })
})

describe('creditNote', () => {
  it('announces the reset of a period-bound pack (Scale)', () => {
    const note = creditNote(tessera, scalePeriod.periodEnd, { refill: 55000n })
    expect(note).toMatch(
      new RegExp(`^Unused credit expires Oct 29${year} · resets to \\$550\\.00$`)
    )
  })

  it('announces the rollover of included credits (Studio)', () => {
    const studio = wallet({
      currency: 'HAL',
      creditUnit: {
        id: 'u1',
        code: 'HAL',
        name: 'Halyard credit',
        pluralName: 'Halyard credits',
        precision: 0,
      },
      grants: [grant({ remaining: 5000n, expiresAt: '2026-10-16T00:00:00' })],
    })
    const note = creditNote(studio, '2026-10-16T00:00:00Z', { rollover: { max: 1000n } })
    expect(note).toMatch(
      new RegExp(`^Included credits expire Oct 16${year} · up to 1,000 roll over$`)
    )
  })

  it('names the soonest free credit to expire with its own amount', () => {
    const bonus = wallet({
      grants: [
        grant({
          remaining: 1000n,
          expiresAt: '2026-10-10T00:00:00',
          type: CreditGrantType.PROMOTIONAL,
        }),
        grant({
          remaining: 4000n,
          expiresAt: '2026-10-20T00:00:00',
          type: CreditGrantType.PROMOTIONAL,
        }),
      ],
    })
    expect(creditNote(bonus, scalePeriod.periodEnd, {})).toMatch(
      new RegExp(`^\\$10\\.00 free credit expires Oct 10${year}$`)
    )
  })
})

describe('idle wallets', () => {
  it('collapses an untouched wallet to its grant (Lumen account credit)', () => {
    const service = wallet({
      available: 2500n,
      live: live(2500n, 0n),
      grants: [
        grant({ remaining: 2500n, reason: 'Service credit', expiresAt: '2027-01-01T00:00:00' }),
      ],
    })
    expect(isIdleWallet(service, 0n, undefined)).toBe(true)
    expect(idleWalletDetail(service)).toMatch(/^Service credit · expires Jan 1(, 2027)?$/)
    expect(isIdleWallet(tessera, 15000n, undefined)).toBe(false)
  })
})

describe('nextInvoiceTitle', () => {
  it('titles an ending the final invoice, and names the plan a change moves to', () => {
    expect(nextInvoiceTitle({ change: UpcomingInvoiceChange.ENDS }, true)).toEqual({
      title: 'Final invoice',
    })
    expect(
      nextInvoiceTitle({ change: UpcomingInvoiceChange.PLAN_CHANGE, newPlanName: 'Scale' }, false)
    ).toEqual({ title: 'Next invoice', after: 'after the change to Scale' })
    expect(nextInvoiceTitle({ change: UpcomingInvoiceChange.PAUSE }, false)).toEqual({
      title: 'Last invoice before the pause',
    })
    expect(nextInvoiceTitle({ change: UpcomingInvoiceChange.NONE }, true)).toEqual({
      title: 'First invoice',
    })
  })
})

describe('the next invoice in every view', () => {
  it('is dated on its issue day, after the grace period, when the server sets one', () => {
    expect(issueDay({ invoiceDate: '2026-11-16', chargeDate: '2026-11-17' })).toBe('2026-11-17')
    expect(issueDay({ invoiceDate: '2026-11-16', chargeDate: '' })).toBe('2026-11-16')
  })

  it('shows what is left to pay after credit, approximate when the credit is shared', () => {
    const money = (minor: bigint) => `$${minor / 100n}`
    expect(nextInvoiceAmount({ amountDue: 50000n, creditEstimateShared: false }, money)).toBe(
      '$500'
    )
    expect(nextInvoiceAmount({ amountDue: 50000n, creditEstimateShared: true }, money)).toBe(
      '≈ $500'
    )
  })
})

describe('outstandingFact', () => {
  it('shows what is open with its due date, or what is overdue', () => {
    const open = create(SubscriptionReceivablesSchema, {
      currency: 'USD',
      openAmount: 540000n,
      openCount: 1,
      nextDueAt: '2026-10-31T00:00:00',
    })
    expect(outstandingFact(open)?.text).toMatch(new RegExp(`^\\$5,400\\.00 · due Oct 31${year}$`))
    const overdue = create(SubscriptionReceivablesSchema, {
      currency: 'USD',
      openAmount: 50000n,
      overdueAmount: 50000n,
    })
    expect(outstandingFact(overdue)).toEqual({ text: '$500.00 overdue', tone: 'destructive' })
    expect(
      outstandingFact(create(SubscriptionReceivablesSchema, { currency: 'USD' }))
    ).toBeUndefined()
  })
})

describe('overdueDetails', () => {
  const receivables = (count: number, failed: boolean) =>
    create(SubscriptionReceivablesSchema, {
      currency: 'USD',
      openAmount: 40000n,
      overdueAmount: 40000n,
      overdueCount: count,
      oldestOverdue: create(OverdueInvoiceSchema, {
        invoiceNumber: 'INV-0034',
        dueAt: '2026-10-08T00:00:00',
        ...(failed && {
          lastFailureAt: '2026-10-08T09:00:00',
          lastFailureReason: 'card_declined',
          nextRetryAt: '2026-10-11T09:00:00',
        }),
      }),
    })

  it('names the oldest overdue invoice, then its failed payment and next retry', () => {
    const lines = overdueDetails(receivables(4, true), asOf)
    expect(lines[0]).toBe('4 invoices overdue · oldest INV-0034, 1 day late.')
    expect(lines[1]).toMatch(
      new RegExp(`^Last payment attempt failed Oct 8${year}: card declined\\.$`)
    )
    expect(lines[2]).toMatch(new RegExp(`^Next retry Oct 11${year}\\.$`))
  })

  it('says only how late a single invoice is when no payment failed', () => {
    expect(overdueDetails(receivables(1, false), asOf)).toEqual(['INV-0034 is 1 day late.'])
  })

  it('is empty when nothing is overdue', () => {
    expect(
      overdueDetails(create(SubscriptionReceivablesSchema, { currency: 'USD' }), asOf)
    ).toEqual([])
  })
})

describe('primaryAction', () => {
  const base = {
    status: SubscriptionStatus.ACTIVE,
    pendingCheckout: false,
    hasWallet: false,
    isFreePlan: false,
    hasMinimum: false,
  }
  it('follows the subscription state', () => {
    expect(primaryAction({ ...base, hasWallet: true })).toBe('add-credit')
    expect(primaryAction({ ...base, isFreePlan: true })).toBe('change-plan')
    expect(primaryAction({ ...base, hasMinimum: true })).toBe('amend')
    expect(
      primaryAction({ ...base, status: SubscriptionStatus.PENDING, pendingCheckout: true })
    ).toBe('checkout')
    expect(
      primaryAction({ ...base, status: SubscriptionStatus.CANCELED, hasWallet: true })
    ).toBeUndefined()
  })
})

describe('averageUsage', () => {
  it('averages the latest periods, beyond-credit usage included (Tessera: $485)', () => {
    expect(averageUsage([38000n, 61000n, 46500n])).toBe(48500n)
    expect(averageUsage([])).toBeUndefined()
  })
})

describe('sinceLabel', () => {
  it('reads the time alone today, else the day and time', () => {
    const now = new Date(2026, 9, 9, 16, 20)
    expect(sinceLabel(new Date(2026, 9, 9, 11, 20), now)).toBe('since 11:20 today')
    expect(sinceLabel(new Date(2026, 9, 7, 9, 5), now)).toBe('since Oct 7, 09:05')
  })
})

describe('walletlessCredit', () => {
  const pending = create(PendingInvoiceCreditSchema, {
    invoiceId: 'inv1',
    invoiceNumber: 'INV-0001',
    currency: 'USD',
    amount: 55000n,
  })

  it('shows credit waiting on payment when no wallet holds it yet (new Scale subscription)', () => {
    const rows = walletlessCredit([{ key: 'USD', total: 55000n }], [pending], [])
    expect(rows).toHaveLength(1)
    expect(rows[0].pending.map(p => [p.invoiceNumber, p.amount])).toEqual([['INV-0001', 55000n]])
    expect(rows[0].nextAdds).toBe(55000n)
  })

  it('leaves credit a wallet already holds to that wallet', () => {
    expect(walletlessCredit([{ key: 'USD', total: 55000n }], [pending], [tessera])).toEqual([])
  })
})

describe('packSplit', () => {
  it('splits a discounted pack on what was paid after discounts', () => {
    // $550 of credit; list $500, a 10% coupon leaves $450 paid.
    expect(packSplit(55000n, 45000n)).toEqual({ paid: 45000n, bonus: 10000n })
    expect(packSplit(55000n, 55000n)).toBeUndefined()
    expect(packSplit(55000n, 0n)).toBeUndefined()
  })
})

describe('packRollover', () => {
  const included = (quantity: number, maxAmount?: string) => ({
    period: SubscriptionFeeBillingPeriod.MONTHLY,
    quantity: 2,
    fee: create(SubscriptionFeeSchema, {
      creditUnit: { creditUnitId: 'u1', code: 'HAL', precision: 0 },
      fee: {
        case: 'credit',
        value: {
          rate: '0',
          quantity,
          total: '0',
          creditAmount: '5000',
          creditTotal: '5000',
          rollover: { maxAmount },
        },
      },
    }),
  })

  it('scales the cap per pack like the credit it grants', () => {
    expect(packRollover([included(3, '1000')], 'USD', 'u1', 0)).toEqual({ max: 6000n })
    expect(packRollover([included(1, '1000'), included(1, undefined)], 'USD', 'u1', 0)).toEqual({})
    expect(packRollover([included(1, '1000')], 'USD', 'USD', 0)).toBeUndefined()
  })
})

describe('invoiceAfterCredit', () => {
  const issued = (init: MessageInitShape<typeof InvoiceSchema>) =>
    create(InvoiceSchema, { status: InvoiceStatus.FINALIZED, currency: 'USD', ...init })

  it('reconciles a row: total − paid with credit = charged, a coupon already in the total', () => {
    // $1,240 usage + $500 fee − $500 coupon + $3,760 shortfall + $400 tax = $5,400.
    const paid = issued({
      total: 540000n,
      minimumShortfall: 376000n,
      appliedCredits: 40000n,
      amountDue: 0n,
      paymentStatus: InvoicePaymentStatus.PAID,
    })
    expect(invoiceAfterCredit(paid)).toEqual({ kind: 'settled', amount: 500000n })
  })

  it('shows what a partially paid invoice still owes', () => {
    const partial = issued({
      total: 50000n,
      amountDue: 20000n,
      paymentStatus: InvoicePaymentStatus.PARTIALLY_PAID,
    })
    expect(invoiceAfterCredit(partial)).toEqual({ kind: 'due', amount: 20000n })
  })
})

describe('limitRefused', () => {
  it('refuses at the limit plus its grace, not before', () => {
    expect(limitRefused('1020', '1000', 10)).toBe(false)
    expect(limitRefused('1100', '1000', 10)).toBe(true)
    expect(limitRefused('995', '1000')).toBe(false)
    expect(limitRefused('1000', '1000')).toBe(true)
  })
})
