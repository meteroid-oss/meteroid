import { seatUnit } from '@/features/plans/canvas/billingWords'
import { FeeStructure_BillingType, FeeStructure_UsageModel } from '@/rpc/api/prices/v1/models_pb'

import type { FeeStructure } from '@/rpc/api/prices/v1/models_pb'

/** A word of the line, or the metric it reads (shown as a link). */
export type ChargeWord = string | { metricId: string; name: string }

const USAGE_MODELS: Record<FeeStructure_UsageModel, string> = {
  [FeeStructure_UsageModel.PER_UNIT]: 'per unit',
  [FeeStructure_UsageModel.TIERED]: 'tiered',
  [FeeStructure_UsageModel.VOLUME]: 'volume',
  [FeeStructure_UsageModel.PACKAGE]: 'package',
  [FeeStructure_UsageModel.MATRIX]: 'by dimension',
  [FeeStructure_UsageModel.DIMENSION_RATES]: 'by dimension',
  [FeeStructure_UsageModel.RATE_CARD]: 'by rate card',
  [FeeStructure_UsageModel.PERCENTAGE]: 'percentage of amount',
  [FeeStructure_UsageModel.DYNAMIC]: 'price from events',
}

/** What a product charges for, in plain words: "Seats · counts seat", "Usage · Tokens · tiered". */
export function chargesForWords(
  feeStructure: FeeStructure | undefined,
  metricName?: string
): ChargeWord[] {
  const structure = feeStructure?.structure
  const metric = (metricId: string): ChargeWord => ({ metricId, name: metricName || 'its metric' })
  switch (structure?.case) {
    case 'rate':
      return ['Flat fee']
    case 'slot':
      return ['Seats', `counts ${seatUnit({ product: { feeStructure } }).singular}`]
    case 'capacity':
      return ['Committed usage', metric(structure.value.metricId)]
    case 'usage':
      return [
        'Usage',
        metric(structure.value.metricId),
        USAGE_MODELS[structure.value.model] ?? 'priced on usage',
      ]
    case 'extraRecurring':
      return [
        'Recurring fee',
        structure.value.billingType === FeeStructure_BillingType.ARREAR
          ? 'billed in arrears'
          : 'billed in advance',
      ]
    case 'oneTime':
      return ['One-time fee']
    case 'credit':
      return ['Credit pack', 'funds invoice credit once paid']
    default:
      return []
  }
}

/**
 * How the charges using a product bill it, when they differ: "Flat fee or Seats". A product has
 * no structure of its own; each plan charge and add-on bills it its own way.
 */
export function chargeKindsWords(structures: FeeStructure[]): string {
  const kinds = [
    ...new Set(
      structures
        .map(structure => chargesForWords(structure)[0])
        .filter((word): word is string => typeof word === 'string')
    ),
  ]
  if (kinds.length < 2) return kinds[0] ?? ''
  return `${kinds.slice(0, -1).join(', ')} or ${kinds[kinds.length - 1].toLowerCase()}`
}

/** "message-square" → "Message square". */
export const iconLabel = (icon: string | undefined) => {
  const name = (icon || 'box').replace(/-/g, ' ')
  return name[0].toUpperCase() + name.slice(1)
}

/** A color token as a word: "Blue"; none is "Neutral". */
export const colorLabel = (color: string | undefined) =>
  color ? color[0].toUpperCase() + color.slice(1) : 'Neutral'
