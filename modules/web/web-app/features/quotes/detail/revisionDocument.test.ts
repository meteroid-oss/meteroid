import { describe, expect, it } from 'vitest'

import { parseRevisionDocument } from './revisionDocument'

describe('parseRevisionDocument trial', () => {
  it('reads a trial decided at the end', () => {
    const summary = parseRevisionDocument(
      JSON.stringify({ trial_duration_days: 14, trial_free: true, trial_end_action: 'MANUAL' })
    )
    expect(summary?.trialDays).toBe(14)
    expect(summary?.trialManual).toBe(true)
  })

  it('reads a document without an end action as converting', () => {
    const summary = parseRevisionDocument(JSON.stringify({ trial_duration_days: 14 }))
    expect(summary?.trialManual).toBe(false)
  })
})
