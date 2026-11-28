import { toPricingTypeFromFeeType } from '@/features/pricing/conversions'
import { pricingDefaults } from '@/features/pricing/schemas'

import type { ComponentFeeType } from '@/features/pricing/conversions'
import type { PricingType } from '@/features/pricing/schemas'

// A charge is one answer to each of: how many, how much, when (price-model-v3 §2.1-2.4).
// The backend still stores the seven fee structures, so a shape compiles into one of them.

export type QuantitySource = 'fixed' | 'seats' | 'metered'
export type RatingModel =
  | 'flat'
  | 'per_unit'
  | 'tiered'
  | 'volume'
  | 'package'
  | 'by_dimension'
  | 'percentage'
  | 'dynamic'
  | 'included'
  | 'credit_pack'
export type Recurrence = 'recurring' | 'one_time'
/** Where usage priced by dimension takes its rates: the plan's own table, or a shared rate card. */
export type RateSource = 'plan' | 'card'
export type Billing = 'advance' | 'arrears'
export type Cadence = 'MONTHLY' | 'QUARTERLY' | 'SEMIANNUAL' | 'ANNUAL'

export interface ChargeShape {
  source: QuantitySource
  model: RatingModel
  recurrence: Recurrence
  billing: Billing
}

/**
 * What a charge whose shape is fixed already is (a subscription's fee overridden, an add-on
 * edited): its shape must compile to it. Not its usage model: the server prices a usage charge's
 * metric with any of them. A product never locks a charge: each charge owns its shape.
 */
export interface ChargeLock {
  feeType: ComponentFeeType
  billingType?: 'ADVANCE' | 'ARREAR'
}

export type ChargeValues = Record<string, unknown>

export interface ChargeTarget {
  feeType: ComponentFeeType
  usageModel?: string
  billingType?: 'ADVANCE' | 'ARREAR'
  /** The rating model kind: every price of one charge uses the same. */
  kind: string
}

export interface CompiledCharge {
  feeType: ComponentFeeType
  pricingType: PricingType
  formData: Record<string, unknown>
}

export const SOURCE_LABELS: Record<QuantitySource, string> = {
  fixed: 'Fixed count',
  seats: 'Seats',
  metered: 'Usage',
}

export const MODEL_LABELS: Record<RatingModel, string> = {
  flat: 'Flat amount',
  per_unit: 'Per unit',
  tiered: 'Graduated',
  volume: 'Volume',
  package: 'Package',
  by_dimension: 'By dimension',
  percentage: 'Percentage',
  dynamic: 'Price from events',
  included: 'Included + overage',
  credit_pack: 'Credit pack',
}

export function modelsFor(source: QuantitySource): RatingModel[] {
  switch (source) {
    case 'fixed':
      return ['flat', 'per_unit', 'tiered', 'volume', 'package', 'credit_pack']
    case 'seats':
      return ['per_unit', 'tiered', 'volume', 'package']
    case 'metered':
      return [
        'per_unit',
        'tiered',
        'volume',
        'package',
        'by_dimension',
        'percentage',
        'dynamic',
        'included',
      ]
  }
}

export function recurrencesFor(shape: Pick<ChargeShape, 'source'>): Recurrence[] {
  return shape.source === 'fixed' ? ['recurring', 'one_time'] : ['recurring']
}

/** Empty for one-time charges; a single entry when the timing is implied. */
export function billingsFor(shape: Omit<ChargeShape, 'billing'>): Billing[] {
  if (shape.recurrence === 'one_time') return []
  switch (shape.source) {
    case 'fixed':
      return shape.model === 'credit_pack' ? ['advance'] : ['advance', 'arrears']
    case 'seats':
      return ['advance']
    case 'metered':
      return shape.model === 'included' ? ['advance'] : ['arrears']
  }
}

/** The closest valid shape: invalid combinations are never offered. */
export function normalizeShape(shape: ChargeShape): ChargeShape {
  const models = modelsFor(shape.source)
  const model = models.includes(shape.model) ? shape.model : models[0]
  const recurrences = recurrencesFor(shape)
  const recurrence = recurrences.includes(shape.recurrence) ? shape.recurrence : recurrences[0]
  const billings = billingsFor({ source: shape.source, model, recurrence })
  const billing =
    billings.length === 0 ? 'advance' : billings.includes(shape.billing) ? shape.billing : billings[0]
  return { source: shape.source, model, recurrence, billing }
}

export function chargeTarget(shape: ChargeShape, lock?: ChargeLock): ChargeTarget {
  const s = normalizeShape(shape)
  switch (s.source) {
    case 'fixed': {
      if (s.model === 'credit_pack') return { feeType: 'credit', kind: 'credit' }
      const quantityModel = s.model === 'flat' ? 'per_unit' : s.model
      if (s.recurrence === 'one_time') return { feeType: 'oneTime', kind: quantityModel }
      // A flat fee in advance is a rate, unless the product is already an extra recurring fee.
      if (s.model === 'flat' && s.billing === 'advance' && lock?.feeType !== 'extraRecurring') {
        return { feeType: 'rate', kind: 'rate' }
      }
      return {
        feeType: 'extraRecurring',
        billingType: s.billing === 'advance' ? 'ADVANCE' : 'ARREAR',
        kind: quantityModel,
      }
    }
    case 'seats':
      return { feeType: 'slot', kind: s.model }
    case 'metered':
      if (s.model === 'included') return { feeType: 'capacity', kind: 'capacity' }
      return { feeType: 'usage', usageModel: s.model, kind: s.model }
  }
}

export function shapeFits(shape: ChargeShape, lock?: ChargeLock): boolean {
  if (!lock) return true
  const target = chargeTarget(shape, lock)
  if (target.feeType !== lock.feeType) return false
  if (lock.billingType && target.billingType && target.billingType !== lock.billingType) return false
  return true
}

/**
 * The shape after one answer changes, adjusted so it stays valid and fits the lock; undefined
 * when no valid shape with that answer fits.
 */
export function bestFit(
  base: ChargeShape,
  change: Partial<ChargeShape>,
  lock?: ChargeLock
): ChargeShape | undefined {
  const keys = Object.keys(change) as (keyof ChargeShape)[]
  const keeps = (shape: ChargeShape) => keys.every(key => shape[key] === change[key])
  const first = normalizeShape({ ...base, ...change })
  if (keeps(first) && shapeFits(first, lock)) return first
  for (const model of modelsFor(first.source)) {
    for (const recurrence of recurrencesFor(first)) {
      const billings = billingsFor({ source: first.source, model, recurrence })
      for (const billing of billings.length ? billings : (['advance'] as Billing[])) {
        const candidate = normalizeShape({ source: first.source, model, recurrence, billing, ...change })
        if (keeps(candidate) && shapeFits(candidate, lock)) return candidate
      }
    }
  }
  return undefined
}

const pick = (values: ChargeValues, keys: string[]) =>
  Object.fromEntries(keys.filter(key => values[key] !== undefined).map(key => [key, values[key]]))

const CREDIT_UNIT_KEYS = ['creditUnitId', 'creditUnitRate', 'creditRateOptional', 'creditUnitValued']
const CREDIT_PACK_KEYS = [
  'unitPrice',
  'quantity',
  'creditAmount',
  'grantCreditUnitId',
  'expiryUnit',
  'expiryValue',
  'rolloverEnabled',
  'rolloverMaxAmount',
  'rolloverExpiryUnit',
  'rolloverExpiryValue',
]

/** The usage model rates by dimension store: the plan's own rates, or a rate card's. */
export const rateSourceModel = (values: ChargeValues) =>
  values.rateSource === 'card' ? 'rate_card' : 'dimension_rates'

function modelFields(model: RatingModel, values: ChargeValues): ChargeValues {
  switch (model) {
    case 'tiered':
    case 'volume':
      return { rows: values.rows ?? [] }
    case 'package':
      return pick(values, ['packagePrice', 'blockSize'])
    case 'percentage':
      return pick(values, ['percent'])
    case 'by_dimension':
      return values.rateSource === 'card'
        ? pick(values, ['rateCardId', 'markup'])
        : {
            dimensions: values.dimensions ?? [],
            keyRates: values.keyRates ?? [],
            ...pick(values, ['unitSize', 'onUnmatched']),
          }
    default:
      return {}
  }
}

/** A fixed count: a flat amount is one unit, a model prices the count (unit price ignored). */
function fixedPricing(model: RatingModel, values: ChargeValues): ChargeValues {
  if (model === 'flat') return { unitPrice: values.rate, quantity: 1, quantityModel: 'per_unit' }
  if (model === 'per_unit') {
    return { unitPrice: values.unitPrice, quantity: values.quantity, quantityModel: 'per_unit' }
  }
  return {
    unitPrice: '0',
    quantity: values.quantity,
    quantityModel: model,
    ...modelFields(model, values),
  }
}

/** The fee type, pricing type and form data the existing builders (conversions.ts) take. */
export function compileCharge(
  shape: ChargeShape,
  values: ChargeValues,
  lock?: ChargeLock
): CompiledCharge {
  const s = normalizeShape(shape)
  const target = chargeTarget(s, lock)
  const usageModel = s.model === 'by_dimension' ? rateSourceModel(values) : target.usageModel
  const term = (values.term as Cadence | undefined) ?? 'MONTHLY'
  const formData = ((): Record<string, unknown> => {
    switch (target.feeType) {
      case 'rate':
        return { term, rate: values.rate }
      case 'extraRecurring':
        return { term, billingType: target.billingType, ...fixedPricing(s.model, values) }
      case 'oneTime':
        return fixedPricing(s.model, values)
      case 'credit':
        return { term, ...pick(values, CREDIT_PACK_KEYS), once: s.recurrence === 'one_time' }
      case 'slot':
        return {
          term,
          slotUnitName: (values.slotUnitName as string | undefined) || 'Seats',
          quantityId: values.quantityId,
          seatModel: s.model,
          unitRate: s.model === 'per_unit' ? values.unitPrice : undefined,
          ...modelFields(s.model, values),
          ...pick(values, ['minSlots', 'maxSlots']),
        }
      case 'usage':
        return {
          term,
          metricId: values.metricId,
          usageModel,
          ...(s.model === 'per_unit' ? { unitPrice: values.unitPrice } : {}),
          ...modelFields(s.model, values),
          ...pick(values, CREDIT_UNIT_KEYS),
        }
      case 'capacity':
        return {
          term,
          metricId: values.metricId,
          thresholds: values.thresholds ?? [],
          ...pick(values, CREDIT_UNIT_KEYS),
        }
    }
  })()
  return {
    feeType: target.feeType,
    pricingType: toPricingTypeFromFeeType(target.feeType, usageModel),
    formData,
  }
}

const isOne = (value: unknown) => Number(value ?? 1) === 1

/** The composer state of a stored charge: its fee type and form data (pricesToFormData). */
export function decompileCharge(
  feeType: ComponentFeeType,
  formData: Record<string, unknown>
): { shape: ChargeShape; values: ChargeValues } {
  const term = (formData.term as Cadence | undefined) ?? 'MONTHLY'
  const base = { ...formData, term }
  const fixed = (
    recurrence: Recurrence,
    billing: Billing
  ): { shape: ChargeShape; values: ChargeValues } => {
    const quantityModel = (formData.quantityModel as RatingModel | undefined) ?? 'per_unit'
    if (quantityModel === 'per_unit' && isOne(formData.quantity)) {
      return {
        shape: { source: 'fixed', model: 'flat', recurrence, billing },
        values: { ...base, rate: formData.unitPrice, quantity: 1 },
      }
    }
    return { shape: { source: 'fixed', model: quantityModel, recurrence, billing }, values: base }
  }
  switch (feeType) {
    case 'rate':
      return {
        shape: { source: 'fixed', model: 'flat', recurrence: 'recurring', billing: 'advance' },
        values: base,
      }
    case 'extraRecurring':
      return fixed('recurring', formData.billingType === 'ARREAR' ? 'arrears' : 'advance')
    case 'oneTime':
      return fixed('one_time', 'advance')
    case 'credit':
      return {
        shape: {
          source: 'fixed',
          model: 'credit_pack',
          recurrence: formData.once ? 'one_time' : 'recurring',
          billing: 'advance',
        },
        values: base,
      }
    case 'slot':
      return {
        shape: {
          source: 'seats',
          model: (formData.seatModel as RatingModel | undefined) ?? 'per_unit',
          recurrence: 'recurring',
          billing: 'advance',
        },
        values: { ...base, unitPrice: formData.unitRate ?? formData.unitPrice },
      }
    case 'usage': {
      const usageModel = (formData.usageModel as string | undefined) ?? 'per_unit'
      // A rate card and the plan's own rates (a matrix stored before them) are rates by dimension.
      const byDimension = ['rate_card', 'dimension_rates', 'matrix'].includes(usageModel)
      return {
        shape: {
          source: 'metered',
          model: byDimension ? 'by_dimension' : (usageModel as RatingModel),
          recurrence: 'recurring',
          billing: 'arrears',
        },
        values: byDimension
          ? { ...base, rateSource: usageModel === 'rate_card' ? 'card' : 'plan' }
          : base,
      }
    }
    case 'capacity': {
      // An override stores its one level flat (rate, included, overageRate): read as a level.
      const flat = formData.thresholds === undefined && formData.rate !== undefined
      // A plan's levels, smallest first, as the customer picks among them.
      const levels = Array.isArray(formData.thresholds)
        ? [...(formData.thresholds as { included?: unknown }[])].sort(
            (a, b) => Number(a.included ?? 0) - Number(b.included ?? 0)
          )
        : formData.thresholds
      return {
        shape: { source: 'metered', model: 'included', recurrence: 'recurring', billing: 'advance' },
        values: flat
          ? {
              ...base,
              thresholds: [
                {
                  included: Number(formData.included ?? 0),
                  rate: formData.rate,
                  overageRate: formData.overageRate ?? '0.00000000',
                },
              ],
            }
          : { ...base, thresholds: levels },
      }
    }
  }
}

/** Starting values of a shape's fields. */
export function chargeDefaults(shape: ChargeShape): ChargeValues {
  const common: ChargeValues = { term: 'MONTHLY' }
  if (shape.source === 'seats') common.slotUnitName = 'Seats'
  if (shape.source === 'metered') common.metricId = ''
  switch (shape.model) {
    case 'flat':
      return { ...common, rate: '0.00' }
    case 'per_unit':
      return { ...common, unitPrice: '0.00', quantity: 1 }
    case 'tiered':
    case 'volume':
      return { ...common, quantity: 1, ...pricingDefaults('tiered') }
    case 'package':
      return { ...common, quantity: 1, packagePrice: '0.00', blockSize: 1 }
    case 'by_dimension':
      return {
        ...common,
        rateSource: 'plan',
        ...pricingDefaults('dimensionRates'),
        ...pricingDefaults('rateCard'),
      }
    case 'percentage':
      return { ...common, ...pricingDefaults('percentage') }
    case 'dynamic':
      return common
    case 'included':
      return { ...common, thresholds: [{ included: 0, rate: '0.00', overageRate: '0.00000000' }] }
    case 'credit_pack':
      return { ...common, ...pricingDefaults('credit') }
  }
}

const isBlank = (value: unknown) =>
  value === undefined || value === '' || (Array.isArray(value) && value.length === 0)

/** The values kept across a change of shape, with the new shape's missing fields filled in. */
export function valuesForShape(
  next: ChargeShape,
  current: ChargeValues,
  _previous?: ChargeShape
): ChargeValues {
  const defaults = chargeDefaults(next)
  const out: ChargeValues = { ...current }
  for (const [key, value] of Object.entries(defaults)) {
    if (isBlank(out[key])) out[key] = value
  }
  return out
}

export type PresetId =
  | 'platform'
  | 'seat'
  | 'usage'
  | 'usage_included'
  | 'setup'
  | 'credit'
  | 'include'

export interface ChargePreset {
  id: PresetId
  label: string
  /** None: the product is included with no price. */
  shape?: ChargeShape
}

export const PRESETS: ChargePreset[] = [
  {
    id: 'platform',
    label: 'Platform fee',
    shape: { source: 'fixed', model: 'flat', recurrence: 'recurring', billing: 'advance' },
  },
  {
    id: 'seat',
    label: 'Per seat',
    shape: { source: 'seats', model: 'per_unit', recurrence: 'recurring', billing: 'advance' },
  },
  {
    id: 'usage',
    label: 'Usage',
    shape: { source: 'metered', model: 'per_unit', recurrence: 'recurring', billing: 'arrears' },
  },
  {
    id: 'usage_included',
    label: 'Usage with included amount',
    shape: { source: 'metered', model: 'included', recurrence: 'recurring', billing: 'advance' },
  },
  {
    id: 'setup',
    label: 'Setup fee',
    shape: { source: 'fixed', model: 'flat', recurrence: 'one_time', billing: 'advance' },
  },
  {
    id: 'credit',
    label: 'Credit pack',
    shape: { source: 'fixed', model: 'credit_pack', recurrence: 'recurring', billing: 'advance' },
  },
  { id: 'include', label: 'Include without price' },
]

export const sameShape = (a: ChargeShape, b: ChargeShape) => {
  const x = normalizeShape(a)
  const y = normalizeShape(b)
  return (
    x.source === y.source &&
    x.model === y.model &&
    x.recurrence === y.recurrence &&
    x.billing === y.billing
  )
}

export function presetOf(shape: ChargeShape): PresetId | undefined {
  return PRESETS.find(preset => preset.shape && sameShape(preset.shape, shape))?.id
}

/**
 * The prices an edit keeps: other currencies always; in the edited currency, the other cadences
 * while the rating model kind is unchanged and the new cadence does not replace them.
 */
export function pricesKeptOnEdit<P extends { id: string; currency: string; cadence: number }>(
  prices: P[],
  edited: P[],
  currency: string,
  newCadence: number,
  sameKind: boolean
): string[] {
  const editedIds = new Set(edited.map(price => price.id))
  return prices
    .filter(price => {
      if (editedIds.has(price.id)) return false
      if (price.currency !== currency) return true
      return sameKind && price.cadence !== newCadence
    })
    .map(price => price.id)
}
