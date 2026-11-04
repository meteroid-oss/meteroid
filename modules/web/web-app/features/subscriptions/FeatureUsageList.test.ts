import { describe, expect, it } from 'vitest'

import { limitBehaviorNote } from './FeatureUsageList'

describe('limitBehaviorNote', () => {
  it('says a soft limit bills overage, and a hard limit blocks', () => {
    expect(limitBehaviorNote({ overageBehavior: 'allow' })).toBe('Soft limit, overage billed')
    expect(limitBehaviorNote({ overageBehavior: 'block', gracePct: 10 })).toBe(
      'Hard limit (+10% grace)'
    )
    expect(limitBehaviorNote({ overageBehavior: 'track' })).toBeUndefined()
  })

  it('never claims overage is billed during a free trial: it says from when it is', () => {
    expect(limitBehaviorNote({ overageBehavior: 'allow' }, new Date(2026, 10, 3))).toBe(
      'Soft limit · trial usage is not billed, overage is billed from 3 Nov'
    )
  })
})
