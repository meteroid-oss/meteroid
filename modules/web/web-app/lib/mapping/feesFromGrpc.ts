import { match } from 'ts-pattern'

import * as grpc from '@/rpc/api/pricecomponents/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import * as api from '../schemas/plans'

// function mapDiscount(grpcDiscount: GrpcStandardDiscount): StandardDiscount | undefined {
//   switch (grpcDiscount.discountType?.case) {
//     case 'amount':
//       return { amount: grpcDiscount.discountType.amount.valueInCents }
//     case 'percent':
//       return { percentage: grpcDiscount.discountType.percent.percentage?.value ?? '0' }
//     default:
//       return undefined
//   }
// }

// function mapPercentDiscount(grpcDiscount: Discount_Percent): PercentDiscount | undefined {
//   return { percentage: grpcDiscount.percentage?.value ?? '0' }
// }

const cadenceMapping: Record<BillingPeriod, api.Cadence> = {
  [BillingPeriod.MONTHLY]: 'MONTHLY',
  [BillingPeriod.QUARTERLY]: 'QUARTERLY',
  [BillingPeriod.ANNUAL]: 'ANNUAL',
}

export const mapCadence = (cadence: BillingPeriod): api.Cadence => cadenceMapping[cadence]

const mapTermFee = (pricing: grpc.Fee_TermFeePricing): api.TermFeePricing => {
  // const discount = termRate.discount ? mapPercentDiscount(termRate.discount) : undefined
  return match<grpc.Fee_TermFeePricing['pricing'], api.TermFeePricing>(pricing.pricing)
    .with({ case: 'termBased' }, ({ value }) => ({
      rates: value.rates.map(rate => ({
        price: rate.price?.value ?? '0',
        term: mapCadence(rate.term) ?? 'MONTHLY',
      })),
    }))
    .with({ case: 'single' }, ({ value }) => {
      return {
        price: value.price?.value ?? '0',
        cadence: mapCadence(value.cadence) ?? 'MONTHLY',
      }
    })
    .otherwise(() => {
      throw new Error('Unknown term fee pricing')
    })
}

const mapRate = (grpcRate: grpc.Fee_SubscriptionRate): api.SubscriptionRate => {
  return { pricing: mapTermFee(grpcRate.pricing!) }
}

const mapCapacityPricing = (pricing: grpc.Fee_Capacity_CapacityPricing): api.CapacityPricing => {
  return match<grpc.Fee_Capacity_CapacityPricing['pricing'], api.CapacityPricing>(pricing.pricing)
    .with({ case: 'termBased' }, ({ value }) => ({
      rates: value.rates.map(rate => ({
        term: mapCadence(rate.term) ?? 'MONTHLY',
        thresholds: rate.thresholds.map(threshold => ({
          includedAmount: Number(threshold.includedAmount),
          price: threshold.price?.value ?? '0',
          perUnitOverage: threshold.perUnitOverage?.value ?? '0',
        })),
      })),
    }))
    .with({ case: 'single' }, ({ value }) => {
      return {
        thresholds: value.thresholds.map(threshold => ({
          includedAmount: Number(threshold.includedAmount),
          price: threshold.price?.value ?? '0',
          perUnitOverage: threshold.perUnitOverage?.value ?? '0',
        })),
      }
    })
    .otherwise(() => {
      throw new Error('Unknown capacity fee pricing')
    })
}

function mapCapacity(grpcCapacity: grpc.Fee_Capacity): api.Capacity {
  return {
    metric: grpcCapacity.metric!,
    pricing: mapCapacityPricing(grpcCapacity.pricing!),
  }
}

const billingTypeMapping: Record<grpc.Fee_BillingType, api.FixedFeePricing['billingType']> = {
  [grpc.Fee_BillingType.ADVANCE]: 'ADVANCE',
  [grpc.Fee_BillingType.ARREAR]: 'ARREAR',
}

const mapFixedFee = (fee: grpc.Fee_FixedFeePricing): api.FixedFeePricing => {
  return {
    billingType: billingTypeMapping[fee.billingType],
    quantity: fee.quantity,
    unitPrice: fee.unitPrice?.value ?? '0',
  }
}

function mapRecurringCharge(grpcScheduled: grpc.Fee_RecurringFixedFee): api.RecurringFixedFee {
  return {
    cadence: mapCadence(grpcScheduled.cadence) ?? 'MONTHLY',
    fee: mapFixedFee(grpcScheduled.fee!),
  }
}

function mapOneTimeFee(grpcOneTime: grpc.Fee_OneTime): api.OneTimeFee {
  return {
    pricing: mapFixedFee(grpcOneTime.pricing!),
  }
}

function mapSlotBasedCharge(grpcSlotBased: grpc.Fee_SlotBased): api.SlotBased {
  return {
    pricing: mapTermFee(grpcSlotBased.pricing!),
    slotUnit: grpcSlotBased.slotUnit!,
    upgradePolicy: match(grpcSlotBased.upgradePolicy)
      .with(grpc.Fee_SlotBased_UpgradePolicy.PRORATED, () => 'PRORATED' as const)
      .otherwise(() => {
        throw new Error('Unknown upgrade policy')
      }),
    downgradePolicy: match(grpcSlotBased.downgradePolicy)
      .with(
        grpc.Fee_SlotBased_DowngradePolicy.REMOVE_AT_END_OF_PERIOD,
        () => 'REMOVE_AT_END_OF_PERIOD' as const
      )
      .otherwise(() => {
        throw new Error('Unknown downgrade policy')
      }),
    minimumCount: grpcSlotBased.minimumCount,
    quota: grpcSlotBased.quota,
  }
}

function mapUsageBasedCharge(grpcUsageBased: grpc.Fee_UsageBased): api.UsageBased {
  const model = match<grpc.UsagePricing_Model['model'], api.UsagePricingModel>(
    // eslint-disable-next-line @typescript-eslint/no-non-null-asserted-optional-chain
    grpcUsageBased.model?.model!
  )
    .with({ case: 'perUnit' }, ({ value }) => ({
      model: 'per_unit',
      data: {
        unitPrice: value.unitPrice?.value ?? '0',
      },
    }))
    .with({ case: 'package' }, ({ value: pkg }) => ({
      model: 'package',
      data: {
        blockSize: pkg.blockSize,
        blockPrice: pkg.blockPrice?.value ?? '0',
      },
    }))
    .with({ case: 'tiered' }, ({ value }) => ({
      model: 'tiered',
      data: {
        rows: value.rows.map(row => ({
          firstUnit: row.firstUnit,
          lastUnit: row.lastUnit,
          unitPrice: row.unitPrice?.value ?? '0',
          flatFee: row.flatFee?.value,
          flatCap: row.flatCap?.value,
        })),
        blockSize: value.blockSize?.blockSize
          ? { blockSize: value.blockSize.blockSize }
          : undefined,
      },
    }))
    .with({ case: 'volume' }, ({ value }) => ({
      model: 'volume',
      data: {
        rows: value.rows.map(row => ({
          firstUnit: row.firstUnit,
          lastUnit: row.lastUnit,
          unitPrice: row.unitPrice?.value ?? '0',
          flatFee: row.flatFee?.value,
          flatCap: row.flatCap?.value,
        })),
        blockSize: value.blockSize?.blockSize
          ? { blockSize: value.blockSize.blockSize }
          : undefined,
      },
    }))
    .otherwise(() => {
      throw new Error('Unknown usage based model')
    })

  return {
    model,
    metric: grpcUsageBased.metric!,
  }
}

export const mapFeeType = (feeType: grpc.Fee_Type): api.FeeType => {
  const mappedFeeType = match<grpc.Fee_Type['fee'], api.FeeType>(feeType.fee)
    .with({ case: 'rate' as const }, ({ value }) => ({ fee: 'rate', data: mapRate(value) }))
    .with({ case: 'usageBased' }, ({ value }) => ({
      fee: 'usage_based',
      data: mapUsageBasedCharge(value),
    }))
    .with({ case: 'slotBased' }, ({ value }) => ({
      fee: 'slot_based',
      data: mapSlotBasedCharge(value),
    }))
    .with({ case: 'capacity' }, ({ value }) => ({
      fee: 'capacity',
      data: mapCapacity(value),
    }))
    .with({ case: 'recurring' }, ({ value }) => ({
      fee: 'recurring' as const,
      data: mapRecurringCharge(value),
    }))
    .with({ case: 'oneTime' }, ({ value }) => ({
      fee: 'one_time' as const,
      data: mapOneTimeFee(value),
    }))
    .otherwise(() => {
      throw new Error('Unknown fee type')
    })

  return mappedFeeType
}

export const mapPriceComponent = (grpcPriceComponent: grpc.PriceComponent): api.PriceComponent => {
  return {
    fee: mapFeeType(grpcPriceComponent.feeType!),
    id: grpcPriceComponent.id,
    name: grpcPriceComponent.name,
    productItem: grpcPriceComponent.productItem,
  }
}
