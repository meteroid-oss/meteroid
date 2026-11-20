import { QuoteSectionPlacement } from '@/rpc/api/quotes/v1/models_pb'

import type { SectionState } from '@/features/quotes/editor/quoteV2'

export const isBeforePricing = (s: SectionState) =>
  s.placement === QuoteSectionPlacement.BEFORE_PRICING

/** Sections in reading order: those before the pricing, then those after it. */
export const readingOrder = (sections: SectionState[]) => [
  ...sections.filter(isBeforePricing),
  ...sections.filter(s => !isBeforePricing(s)),
]

/**
 * Moves a section one step in reading order. Crossing the pricing block flips its placement
 * instead of swapping it with a neighbour.
 */
export function moveSection(sections: SectionState[], index: number, offset: -1 | 1) {
  const ordered = readingOrder(sections)
  const section = ordered[index]
  const beforeCount = ordered.filter(isBeforePricing).length

  if (offset === 1 && isBeforePricing(section) && index === beforeCount - 1) {
    ordered[index] = { ...section, placement: QuoteSectionPlacement.AFTER_PRICING }
    return ordered
  }
  if (offset === -1 && !isBeforePricing(section) && index === beforeCount) {
    ordered[index] = { ...section, placement: QuoteSectionPlacement.BEFORE_PRICING }
    return ordered
  }
  const target = index + offset
  ;[ordered[index], ordered[target]] = [ordered[target], ordered[index]]
  return ordered
}

/** A section always crosses the pricing block; it stops only at either end of the document. */
export function canMoveSection(sections: SectionState[], index: number, offset: -1 | 1) {
  const ordered = readingOrder(sections)
  const section = ordered[index]
  if (!section) return false
  if (offset === -1) return index > 0 || !isBeforePricing(section)
  return index < ordered.length - 1 || isBeforePricing(section)
}
