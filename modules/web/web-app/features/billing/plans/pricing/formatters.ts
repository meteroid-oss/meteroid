import { AppRouterOutput } from '@/lib/schemas'

type PriceComponent = AppRouterOutput['plans']['listPriceComponents'][number]

interface PriceComponentRow {
  name: string
  price: string
  feeType: string
  options: string
}

type PartialPCR = Omit<PriceComponentRow, 'name'>

type Fee = NonNullable<NonNullable<PriceComponent['feeType']>['fee']>

type OneTimeFee = Extract<Fee, { $case: 'oneTimeFee' }>['oneTimeFee']
type RecurringFixedFee = Extract<Fee, { $case: 'recurringFixedFee' }>['recurringFixedFee']
type SlotBasedCharge = Extract<Fee, { $case: 'slotBasedCharge' }>['slotBasedCharge']
type UsageBasedCharge = Extract<Fee, { $case: 'usageBasedCharge' }>['usageBasedCharge']

const formatPrice = (price: string): string => {
  return Number(price).toLocaleString(undefined, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 8,
  })
}
const formatOneTimeFee = (fee: OneTimeFee): PartialPCR => {
  return {
    feeType: 'Fixed charge > One-time',
    price: formatPrice(fee.price),
    options: '',
  }
}

const formatRecurringFixedFee = (fee: RecurringFixedFee): PartialPCR => {
  return {
    feeType: 'Fixed charge > Recurring',
    price: fee.price?.amount?.toString() ?? '',
    options: '',
  }
}

const formatSlotBasedCharge = (fee: SlotBasedCharge): PartialPCR => {
  return {
    feeType: 'Slot-based charge',
    price: fee.price?.amount?.toString() ?? '',
    options: '',
  }
}

type ModelType = NonNullable<NonNullable<UsageBasedCharge['model']>['model']>['$case']
const formatModelType = (model: ModelType | undefined): string => {
  switch (model) {
    case 'perUnit':
      return 'Per unit'
    case 'package':
      return 'Package'
    case 'tiered':
      return 'Tiered'
    case 'volume':
      return 'Volume'
    case 'tieredBps':
      return 'Tiered (BPS)'
    case 'volumeBps':
      return 'Volume (BPS)'
    default:
      return 'invalid'
  }
}

const formatUsageBasedFeeComponent = (fee: UsageBasedCharge): PartialPCR => {
  return {
    feeType: `Usage-based charge > ${formatModelType(fee.model?.model?.$case)}`,
    price: fee.price?.amount?.toString() ?? '',
    options: '',
  }
}

export const formatPriceComponent = (pc: PriceComponent, currency: string) => {
  let partial: PartialPCR | undefined
  // $case: "rate" | "slotBased" | "capacity" | "usageBased" | "scheduled" | "oneTime" | undefined
  switch (pc.feeType?.fee?.$case) {
    case 'oneTimeFee':
      partial = formatOneTimeComponent(pc.feeType?.fee.oneTimeFee)
      break
    case 'recurringFixedFee':
      partial = formatRecurringFixedComponent(pc.feeType?.fee.recurringFixedFee)
      break
    case 'slotBasedCharge':
      partial = formatSlotBasedFeeComponent(pc.feeType?.fee.slotBasedCharge)
      break
    case 'usageBasedCharge':
      partial = formatUsageBasedFeeComponent(pc.feeType?.fee.usageBasedCharge)
      break
    case undefined:
      break
  }
}
