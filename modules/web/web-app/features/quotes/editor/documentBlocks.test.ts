import { describe, expect, it } from 'vitest'

import { QuoteSectionPlacement } from '@/rpc/api/quotes/v1/models_pb'

import { canMoveSection, moveSection } from './documentBlocks'

const { BEFORE_PRICING, AFTER_PRICING } = QuoteSectionPlacement
const section = (title: string, placement: QuoteSectionPlacement) => ({
  title,
  body: '',
  placement,
})

describe('moveSection', () => {
  const sections = [
    section('After', AFTER_PRICING),
    section('Before 1', BEFORE_PRICING),
    section('Before 2', BEFORE_PRICING),
  ]

  it('swaps neighbours on the same side of the pricing', () => {
    expect(moveSection(sections, 1, -1).map(s => s.title)).toEqual([
      'Before 2',
      'Before 1',
      'After',
    ])
  })

  it('moves the last section before the pricing to after it', () => {
    const moved = moveSection(sections, 1, 1)
    expect(moved.map(s => [s.title, s.placement])).toEqual([
      ['Before 1', BEFORE_PRICING],
      ['Before 2', AFTER_PRICING],
      ['After', AFTER_PRICING],
    ])
  })

  it('moves the first section after the pricing to before it', () => {
    const moved = moveSection(sections, 2, -1)
    expect(moved.map(s => [s.title, s.placement])).toEqual([
      ['Before 1', BEFORE_PRICING],
      ['Before 2', BEFORE_PRICING],
      ['After', BEFORE_PRICING],
    ])
  })
})

describe('canMoveSection', () => {
  it('lets a lone section after the pricing move before it', () => {
    const lone = [section('After', AFTER_PRICING)]
    expect(canMoveSection(lone, 0, -1)).toBe(true)
    expect(canMoveSection(lone, 0, 1)).toBe(false)
    expect(moveSection(lone, 0, -1).map(s => s.placement)).toEqual([BEFORE_PRICING])
  })

  it('lets a lone section before the pricing move after it', () => {
    const lone = [section('Before', BEFORE_PRICING)]
    expect(canMoveSection(lone, 0, 1)).toBe(true)
    expect(canMoveSection(lone, 0, -1)).toBe(false)
    expect(moveSection(lone, 0, 1).map(s => s.placement)).toEqual([AFTER_PRICING])
  })

  it('stops only at the ends of the document', () => {
    const sections = [section('Before', BEFORE_PRICING), section('After', AFTER_PRICING)]
    expect(canMoveSection(sections, 0, -1)).toBe(false)
    expect(canMoveSection(sections, 0, 1)).toBe(true)
    expect(canMoveSection(sections, 1, -1)).toBe(true)
    expect(canMoveSection(sections, 1, 1)).toBe(false)
  })
})
