import { PlainMessage } from '@bufbuild/protobuf'
import { match, P } from 'ts-pattern'

import * as grpc from '@/rpc/api/pricecomponents/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import * as api from '../schemas/plans'

// function mapDiscount(discount: StandardDiscount): GrpcStandardDiscount | undefined {
//   if ('amount' in discount) {
//     return {
//       discountType: {
//         case: 'amount',
//         amount: { valueInCents: discount.amount },
//       },
//     }
//   } else {
//     return {
//       discountType: {
//         case: 'percent',
//         percent: { percentage: { value: discount.percentage } },
//       },
//     }
//   }
// }

// function mapPercentDiscount(discount: PercentDiscount): Discount_Percent | undefined {
//   return { percentage: { value: discount.percentage } }
// }

export const mapCadence = (cadence: api.Cadence): BillingPeriod => {
  return match(cadence)
    .with('MONTHLY', () => BillingPeriod.MONTHLY)
    .with('QUARTERLY', () => BillingPeriod.QUARTERLY)
    .with('ANNUAL', () => BillingPeriod.ANNUAL)
    .exhaustive()
}

const mapTermFee = (pricing: api.TermFeePricing): grpc.Fee_TermFeePricing => {
  // const discount = termRate.discount ? mapPercentDiscount(termRate.discount) : undefined
  const plain = match<api.TermFeePricing, PlainMessage<grpc.Fee_TermFeePricing>>(pricing)
    .with({ rates: P.array() }, ({ rates }) => ({
      pricing: {
        case: 'termBased' as const,
        value: {
          rates: rates.map(rate => ({
            price: { value: rate.price },
            term: mapCadence(rate.term),
          })),
        },
      },
    }))
    .with({ cadence: P.any, price: P.any }, ({ cadence, price }) => ({
      pricing: {
        case: 'single' as const,
        value: {
          price: { value: price },
          cadence: mapCadence(cadence),
        },
      },
    }))
    .exhaustive()

  return new grpc.Fee_TermFeePricing(plain)
}

const mapCapacityPricing = (pricing: api.CapacityPricing): grpc.Fee_Capacity_CapacityPricing => {
  // const discount = termRate.discount ? mapPercentDiscount(termRate.discount) : undefined
  const plain = match<api.CapacityPricing, PlainMessage<grpc.Fee_Capacity_CapacityPricing>>(pricing)
    .with({ rates: P.array() }, ({ rates }) => ({
      pricing: {
        case: 'termBased' as const,
        value: {
          rates: rates.map(rate => ({
            thresholds: rate.thresholds.map(threshold => ({
              includedAmount: BigInt(threshold.includedAmount),
              price: { value: threshold.price },
              perUnitOverage: { value: threshold.perUnitOverage },
            })),
            term: mapCadence(rate.term),
          })),
        },
      },
    }))
    .with({ thresholds: P.array() }, ({ thresholds }) => {
      return {
        pricing: {
          case: 'single' as const,
          value: {
            thresholds: thresholds.map(threshold => ({
              includedAmount: BigInt(threshold.includedAmount),
              price: { value: threshold.price },
              perUnitOverage: { value: threshold.perUnitOverage },
            })),
          },
        },
      }
    })
    .exhaustive()

  return new grpc.Fee_Capacity_CapacityPricing(plain)
}

const mapRate = (rate: api.SubscriptionRate): PlainMessage<grpc.Fee_SubscriptionRate> => {
  return {
    pricing: mapTermFee(rate.pricing),
  }
}

const mapCapacity = (capacity: api.Capacity): PlainMessage<grpc.Fee_Capacity> => {
  return {
    metric: {
      id: capacity.metric.id,
      name: capacity.metric.name ?? 'N/A', // TODO
    },
    pricing: mapCapacityPricing(capacity.pricing),
  }
}

const mapFixedFee = (fee: api.FixedFeePricing): PlainMessage<grpc.Fee_FixedFeePricing> => {
  return {
    billingType: match(fee.billingType)
      .with('ARREAR' as const, () => grpc.Fee_BillingType.ARREAR)
      .with('ADVANCE' as const, () => grpc.Fee_BillingType.ADVANCE)
      .exhaustive(),
    quantity: fee.quantity,
    unitPrice: { value: fee.unitPrice },
  }
}

const mapRecurringCharge = (
  charge: api.RecurringFixedFee
): PlainMessage<grpc.Fee_RecurringFixedFee> => {
  return {
    fee: mapFixedFee(charge.fee),
    cadence: mapCadence(charge.cadence),
  }
}

const mapOneTimeFee = (charge: api.OneTimeFee): PlainMessage<grpc.Fee_OneTime> => {
  return { pricing: mapFixedFee(charge.pricing) }
}

const mapSlotBasedCharge = (charge: api.SlotBased): PlainMessage<grpc.Fee_SlotBased> => {
  return {
    pricing: mapTermFee(charge.pricing),
    slotUnit: charge.slotUnit,
    upgradePolicy: grpc.Fee_SlotBased_UpgradePolicy[charge.upgradePolicy],
    downgradePolicy: grpc.Fee_SlotBased_DowngradePolicy[charge.downgradePolicy],
    minimumCount: charge.minimumCount,
    quota: charge.quota,
  }
}

const mapUsageBasedCharge = (charge: api.UsageBased): PlainMessage<grpc.Fee_UsageBased> => {
  let model: PlainMessage<grpc.UsagePricing_Model> | undefined

  switch (charge.model.model) {
    case 'per_unit':
      model = {
        model: {
          case: 'perUnit',
          value: {
            unitPrice: { value: charge.model.data.unitPrice },
          },
        },
      }
      break
    case 'package':
      model = {
        model: {
          case: 'package',
          value: {
            blockSize: charge.model.data.blockSize,
            blockPrice: { value: charge.model.data.blockPrice },
          },
        },
      }
      break
    case 'tiered':
    case 'volume': {
      const rows = charge.model.data.rows.map(tier => ({
        firstUnit: tier.firstUnit,
        lastUnit: tier.lastUnit,
        unitPrice: { value: tier.unitPrice },
        flatFee: tier.flatFee ? { value: tier.flatFee } : undefined,
        flatCap: tier.flatCap ? { value: tier.flatCap } : undefined,
      }))

      const blockSize = charge.model.data.blockSize?.blockSize
        ? { blockSize: charge.model.data.blockSize.blockSize }
        : undefined

      if (charge.model.model === 'tiered') {
        model = {
          model: {
            case: 'tiered',
            value: {
              rows,
              blockSize,
            },
          },
        }
      } else {
        model = {
          model: {
            case: 'volume',
            value: {
              rows,
              blockSize,
            },
          },
        }
      }
      break
    }
  }

  return {
    metric: {
      id: charge.metric.id,
      name: charge.metric.name ?? 'N/A', // TODO
    },
    model: model,
  }
}

export const mapFeeType = (feeType: api.FeeType): grpc.Fee_Type => {
  const mappedFeeType: PlainMessage<grpc.Fee_Type>['fee'] = match<
    api.FeeType,
    PlainMessage<grpc.Fee_Type>['fee']
  >(feeType)
    .with({ fee: 'rate' }, ({ data }) => ({ case: 'rate' as const, value: mapRate(data) }))
    .with({ fee: 'usage_based' }, ({ data }) => ({
      case: 'usageBased' as const,
      value: mapUsageBasedCharge(data),
    }))
    .with({ fee: 'slot_based' }, ({ data }) => ({
      case: 'slotBased' as const,
      value: mapSlotBasedCharge(data),
    }))
    .with({ fee: 'capacity' }, ({ data }) => ({
      case: 'capacity' as const,
      value: mapCapacity(data),
    }))
    .with({ fee: 'recurring' }, ({ data }) => ({
      case: 'recurring' as const,
      value: mapRecurringCharge(data),
    }))
    .with({ fee: 'one_time' }, ({ data }) => ({
      case: 'oneTime' as const,
      value: mapOneTimeFee(data),
    }))
    .exhaustive()

  return new grpc.Fee_Type({ fee: mappedFeeType })
}

//////////////////////
