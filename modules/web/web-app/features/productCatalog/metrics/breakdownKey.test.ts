import { describe, expect, it } from 'vitest'

import { copiedBreakdownKey } from './breakdownKey'

describe('copiedBreakdownKey', () => {
  it('copies the source key when the form shows the field', () => {
    expect(copiedBreakdownKey('project_id', true)).toBe('project_id')
  })

  it('drops the source key when no field shows it', () => {
    expect(copiedBreakdownKey('project_id', false)).toBeUndefined()
  })

  it('starts without a key when the source has none', () => {
    expect(copiedBreakdownKey('', true)).toBeUndefined()
    expect(copiedBreakdownKey(undefined, true)).toBeUndefined()
  })
})
