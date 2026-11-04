import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  CreditAppliesTo,
  CreditGrantKind,
  CreditGroupSchema,
  WalletSchema,
} from '@/rpc/api/wallets/v1/models_pb'

import { creditSplitLine, subscriptionCreditSplit } from './subscriptionWallets'

const group = (init: {
  subscriptionId?: string
  available: bigint
  label: string
  appliesTo?: CreditAppliesTo
}) =>
  create(CreditGroupSchema, {
    kind: init.subscriptionId ? CreditGrantKind.INCLUDED : CreditGrantKind.PREPAID,
    appliesTo: init.appliesTo ?? CreditAppliesTo.ALL,
    ...init,
  })

const dollars = (minor: bigint) => `$${minor / 100n}`

describe('subscriptionCreditSplit', () => {
  const wallet = create(WalletSchema, {
    groups: [
      group({
        subscriptionId: 'pro',
        available: 10000n,
        label: 'Included with Pro',
        appliesTo: CreditAppliesTo.USAGE,
      }),
      group({ subscriptionId: 'basic', available: 400n, label: 'Included with Basic' }),
      group({ available: 5000n, label: 'Purchased credit' }),
    ],
  })

  it("splits the subscription's own credit from shared credit, leaving others' out", () => {
    const split = subscriptionCreditSplit(wallet, 'pro')
    expect(split.own).toBe(10000n)
    expect(split.shared).toBe(5000n)
    expect(creditSplitLine(split, dollars)).toBe(
      'Included with Pro $100 · Customer-wide credit $50'
    )
  })

  it('names the one kind present', () => {
    const own = create(WalletSchema, {
      groups: [group({ subscriptionId: 'pro', available: 10000n, label: 'Included with Pro' })],
    })
    expect(creditSplitLine(subscriptionCreditSplit(own, 'pro'), dollars)).toBe(
      'Included with Pro $100'
    )
    const sharedOnly = subscriptionCreditSplit(wallet, 'team')
    expect(creditSplitLine(sharedOnly, dollars)).toBe('Customer-wide credit $50')
    const alone = create(WalletSchema, {
      groups: [group({ available: 5000n, label: 'Purchased credit' })],
    })
    expect(creditSplitLine(subscriptionCreditSplit(alone, 'pro'), dollars)).toBeUndefined()
  })
})
