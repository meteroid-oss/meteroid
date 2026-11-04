import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  CreditGrantSource,
  CreditGrantType,
  CustomerLedgerEntrySchema,
  LedgerEntryType,
  LiveBalanceSchema,
  WalletSchema,
} from '@/rpc/api/wallets/v1/models_pb'

import { movementDetail, pendingUsageRow, scopedPendingRow } from './CustomerLedgerTable'

describe('movementDetail', () => {
  const granted = (label: string, grantName: string, grantSource: CreditGrantSource) =>
    create(CustomerLedgerEntrySchema, {
      label,
      grantName,
      grantSource,
      grantType: CreditGrantType.PROMOTIONAL,
      entry: { type: LedgerEntryType.GRANTED },
    })

  it('names the grant only when it adds to the movement label', () => {
    expect(
      movementDetail(granted('Purchased credit', 'Purchase credits', CreditGrantSource.PURCHASE))
    ).toBeUndefined()
    expect(
      movementDetail(granted('Automatic top-up', 'Automatic top-up', CreditGrantSource.PURCHASE))
    ).toBeUndefined()
    expect(
      movementDetail(granted('Included with plan', 'Studio pack', CreditGrantSource.INCLUDED))
    ).toBe('Studio pack')
    expect(
      movementDetail(granted('Credit note', 'March outage', CreditGrantSource.CREDIT_NOTE))
    ).toBe('March outage (free)')
  })
})

describe('pendingUsageRow', () => {
  const wallet = (live: { pendingUsage: bigint; availableAfterUsage: bigint; stale?: boolean }) =>
    create(WalletSchema, {
      currency: 'USD',
      available: 10_000n,
      live: create(LiveBalanceSchema, live),
    })

  it('takes the usage not invoiced yet off the credit on account', () => {
    expect(
      pendingUsageRow(wallet({ pendingUsage: 2_500n, availableAfterUsage: 7_500n }))
    ).toMatchObject({ pending: true, amount: -2_500n, balanceAfter: 7_500n, currency: 'USD' })
  })

  it('is left out while the estimate catches up, or with nothing pending', () => {
    expect(
      pendingUsageRow(wallet({ pendingUsage: 2_500n, availableAfterUsage: 7_500n, stale: true }))
    ).toBeUndefined()
    expect(
      pendingUsageRow(wallet({ pendingUsage: 0n, availableAfterUsage: 10_000n }))
    ).toBeUndefined()
    expect(pendingUsageRow(create(WalletSchema, {}))).toBeUndefined()
  })
})

describe('scopedPendingRow', () => {
  const wallet = create(WalletSchema, { currency: 'USD' })
  const balance = (unbilled: bigint, uncovered: bigint) => ({
    unbilled,
    uncovered,
    balance: -9_999n,
  })

  it("takes off the statement only the subscription's own usage that credit pays", () => {
    expect(scopedPendingRow(wallet, balance(1_500n, 300n), 4_200n)).toMatchObject({
      pending: true,
      amount: -1_200n,
      balanceAfter: 3_000n,
      currency: 'USD',
    })
  })

  it('is left out when the subscription used nothing credit pays, whatever it can spend', () => {
    expect(scopedPendingRow(wallet, balance(0n, 0n), 4_200n)).toBeUndefined()
    expect(scopedPendingRow(wallet, balance(500n, 500n), 4_200n)).toBeUndefined()
    expect(scopedPendingRow(wallet, undefined, 4_200n)).toBeUndefined()
  })
})
