import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { SubscriptionGrantSchema } from '@/rpc/api/entitlements/v1/models_pb'

import { limitBehaviorNote, ownGrantNote } from './FeatureUsageList'

describe('limitBehaviorNote', () => {
  it('says whether checks deny at the limit or allow overage', () => {
    expect(limitBehaviorNote({ overageBehavior: 'allow' })).toBe('Overage allowed')
    expect(limitBehaviorNote({ overageBehavior: 'block', gracePct: 10 })).toBe(
      'Deny at limit + 10% grace'
    )
    expect(limitBehaviorNote({ overageBehavior: 'track' })).toBeUndefined()
  })

  it('says until when trial usage is not billed', () => {
    expect(limitBehaviorNote({ overageBehavior: 'allow' }, new Date(2026, 10, 3))).toBe(
      'Overage allowed · trial usage not billed until 3 Nov'
    )
  })
})

describe('ownGrantNote', () => {
  const grant = (limit?: string, path?: string) =>
    create(SubscriptionGrantSchema, {
      value: { value: { case: 'meteredValue', value: { limit, enabled: true } } },
      decidingSubscriptionId: 'sub',
      decidingPlanName: 'Pro',
      decidingResourcePath: path,
    })

  it('says what this subscription grants and which subscription decides', () => {
    expect(ownGrantNote(grant('1000'), '10000')).toBe('Grants 1k; Pro on the customer decides 10k')
    expect(ownGrantNote(grant(undefined, 'site=s1'), '10')).toBe(
      'Grants unlimited; Pro on site=s1 decides 10'
    )
  })

  it('says nothing when this subscription decides', () => {
    expect(ownGrantNote(undefined, '10')).toBeUndefined()
  })
})
