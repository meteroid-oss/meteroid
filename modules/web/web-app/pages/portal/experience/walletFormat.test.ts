import { create } from '@bufbuild/protobuf'
import { timestampFromDate } from '@bufbuild/protobuf/wkt'
import { Code, ConnectError } from '@connectrpc/connect'
import { describe, expect, it } from 'vitest'

import {
  CreditGrantKind,
  CreditGrantViewSchema,
  CreditGroupSchema,
  CreditSource,
  CreditUnitLabelSchema,
  RolloverCapSchema,
} from '@/rpc/portal/wallets/v1/models_pb'

import {
  CREDIT_SOURCE_LABEL,
  creditGroupDetail,
  customerError,
  grantKindLabel,
  liveGrants,
  walletAmount,
  walletTitle,
} from './walletFormat'

const halyard = create(CreditUnitLabelSchema, {
  code: 'HALYARD_CREDITS',
  name: 'Halyard credit',
  pluralName: 'Halyard credits',
  precision: 0,
})
const unitWallet = { currency: 'HALYARD_CREDITS', creditUnit: halyard }

describe('walletAmount', () => {
  it('uses the singular name for exactly one credit', () => {
    expect(walletAmount(1n, unitWallet.currency, halyard)).toBe('1 Halyard credit')
  })

  it('honours a unit with decimals', () => {
    const milli = { ...halyard, precision: 3 }
    expect(walletAmount(1500n, unitWallet.currency, milli)).toBe('1.5 Halyard credits')
    expect(walletAmount(1000n, unitWallet.currency, milli)).toBe('1 Halyard credit')
  })
})

describe('customerError', () => {
  it('shows the server’s wording without the code prefix', () => {
    const cause = new ConnectError('Your card was declined.', Code.FailedPrecondition)
    expect(cause.message).toContain('[failed_precondition]')
    expect(customerError(cause, 'fallback')).toBe('Your card was declined.')
  })

  it('hides internal failures and errors that never reached the server', () => {
    expect(customerError(new ConnectError('db pool exhausted', Code.Internal), 'fallback')).toBe(
      'fallback'
    )
    expect(customerError(new TypeError('Failed to fetch'), 'fallback')).toBe('fallback')
    for (const code of [Code.Unavailable, Code.DeadlineExceeded, Code.Canceled]) {
      expect(customerError(new ConnectError('upstream connect error', code), 'fallback')).toBe(
        'fallback'
      )
    }
  })
})

describe('CREDIT_SOURCE_LABEL', () => {
  it('names bonus credit as such, not as a purchase', () => {
    expect(CREDIT_SOURCE_LABEL[CreditSource.BONUS]).toBe('Free credit')
  })
})

describe('portal grants', () => {
  const included = create(CreditGrantViewSchema, {
    id: 'pack',
    source: CreditSource.INCLUDED,
    amount: 5000n,
    remaining: 1720n,
    scope: { kindLabel: 'Included with Studio' },
  })
  const rolled = create(CreditGrantViewSchema, {
    ...included,
    id: 'rolled',
    source: CreditSource.ROLLED_OVER,
    amount: 620n,
    remaining: 620n,
    remainingAfterUsage: 0n,
  })

  it('marks a rolled-over grant apart from the pack it came from', () => {
    expect(grantKindLabel(included)).toBe('Included with Studio')
    expect(grantKindLabel(rolled)).toBe('Included with Studio · Rolled over from last period')
  })

  it('keeps the grants with credit left after usage, never a scheduled one', () => {
    const scheduled = create(CreditGrantViewSchema, { ...included, id: 'next', scheduled: true })
    expect(liveGrants([included, rolled, scheduled]).map(grant => grant.id)).toEqual(['pack'])
  })
})

describe('credit groups', () => {
  it('words a plan group with its amount, use, reset and rollover', () => {
    const detail = creditGroupDetail(
      create(CreditGroupSchema, {
        kind: CreditGrantKind.INCLUDED,
        label: 'Included with Studio',
        paysFor: 'Pays Studio usage',
        available: 5000n,
        upcoming: 1000n,
        nextLapseAt: timestampFromDate(new Date('2026-11-01T00:00:00Z')),
        rollover: create(RolloverCapSchema, { maxAmount: 1000n }),
        rolloverTerm: '1 month',
      }),
      unitWallet
    )
    expect(detail.amount).toBe('5,000 Halyard credits')
    expect(detail.scheduled).toBe('+ 1,000 Halyard credits scheduled')
    expect(detail.notes[0]).toBe('Pays Studio usage')
    expect(detail.notes[1]).toMatch(/^Resets Nov 1/)
    expect(detail.notes[2]).toBe('Up to 1,000 Halyard credits roll over for 1 month')
  })

  it('titles a wallet by its currency alone', () => {
    expect(walletTitle({ currency: 'USD' })).toBe('USD credit')
    expect(walletTitle(unitWallet)).toBe('Halyard credits')
  })
})
