import { describe, expect, it } from 'vitest'

import { CreditGrantStatus } from '@/rpc/api/wallets/v1/models_pb'

import { grantUsed, isDrained, isUsedUp, usedUpToggleLabel } from './usedUpGrants'

describe('isUsedUp', () => {
  it('folds away depleted and expired grants only', () => {
    expect(isUsedUp({ status: CreditGrantStatus.DEPLETED })).toBe(true)
    expect(isUsedUp({ status: CreditGrantStatus.EXPIRED })).toBe(true)
    expect(isUsedUp({ status: CreditGrantStatus.ACTIVE })).toBe(false)
    expect(isUsedUp({ status: CreditGrantStatus.PENDING })).toBe(false)
  })
})

describe('isDrained', () => {
  const active = { status: CreditGrantStatus.ACTIVE, remaining: 50_000n }

  it('folds away an active grant whose remainder unbilled usage already spends', () => {
    expect(isDrained(active, { unbilled: 50_000n })).toBe(true)
    expect(isDrained(active, { unbilled: 20_000n })).toBe(false)
    expect(isDrained({ ...active, status: CreditGrantStatus.PENDING }, { unbilled: 50_000n })).toBe(
      false
    )
  })

  it('keeps a grant live until its activity is known', () => {
    expect(isDrained(active)).toBe(false)
  })
})

describe('grantUsed', () => {
  it('adds what was invoiced, settled at expiry and is still unbilled', () => {
    expect(grantUsed({ invoiced: 30_000n, settledAtExpiry: 5_000n, unbilled: 15_000n })).toBe(
      50_000n
    )
    expect(grantUsed(undefined)).toBe(0n)
  })
})

describe('usedUpToggleLabel', () => {
  it('counts the used-up grants the server reports', () => {
    expect(usedUpToggleLabel(1, false)).toBe('Show 1 used-up grant')
    expect(usedUpToggleLabel(42, false)).toBe('Show 42 used-up grants')
    expect(usedUpToggleLabel(42, true)).toBe('Hide 42 used-up grants')
  })
})
