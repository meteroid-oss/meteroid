// Mapper file

import { match, P } from 'ts-pattern'

import * as grpc from '@/rpc/api/pricecomponents/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import * as api from '../schemas/plans'

export const mapCadence = (cadence: api.Cadence): BillingPeriod => {
  return match(cadence)
    .with('MONTHLY', () => BillingPeriod.MONTHLY)
    .with('QUARTERLY', () => BillingPeriod.QUARTERLY)
    .with('SEMIANNUAL', () => BillingPeriod.SEMIANNUAL)
    .with('ANNUAL', () => BillingPeriod.ANNUAL)
    .exhaustive()
}

export const mapCadenceFromGrpc = (cadence: BillingPeriod): api.Cadence => {
  switch (cadence) {
    case BillingPeriod.MONTHLY:
      return 'MONTHLY'
    case BillingPeriod.QUARTERLY:
      return 'QUARTERLY'
    case BillingPeriod.SEMIANNUAL:
      return 'SEMIANNUAL'
    case BillingPeriod.ANNUAL:
      return 'ANNUAL'
  }
}

const mapUsageModel = (model: grpc.UsageFee['model']): api.UsagePricingModel => {
  return match<grpc.UsageFee['model'], api.UsagePricingModel>(model)
    .with({ case: 'perUnit' }, ({ value }) => ({
      model: 'per_unit',
      data: { unitPrice: value },
    }))
    .with({ case: P.union('tiered', 'volume') }, ({ case: model, value }) => ({
      model,
      data: {
        rows: value.rows.map(row => ({
          firstUnit: row.firstUnit,
          unitPrice: row.unitPrice,
          flatFee: row.flatFee,
          flatCap: row.flatCap,
        })),
        blockSize: value.blockSize,
      },
    }))
    .with({ case: 'package' }, ({ value }) => ({
      model: 'package',
      data: {
        packagePrice: value.packagePrice,
        blockSize: value.blockSize,
      },
    }))
    .with({ case: 'matrix' }, ({ value }) => ({
      model: 'matrix',
      data: {
        dimensionRates: value.rows.map(rate => ({
          dimension1: rate.dimension1!,
          dimension2: rate.dimension2,
          price: rate.perUnitPrice,
        })),
      },
    }))
    .with({ case: undefined }, () => {
      console.error('Usage model is required')
      return {
        model: 'per_unit',
        data: { unitPrice: '0' },
      }
    })
    .exhaustive()
}

const mapTermRate = (rate: grpc.Fee_TermRate): api.TermRate => ({
  term: mapCadenceFromGrpc(rate.term),
  price: rate.price,
})

const mapRateFee = (fee: grpc.Fee_RateFee): api.RateFee => ({
  rates: fee.rates.map(mapTermRate),
})

const mapSlotFee = (fee: grpc.Fee_SlotFee): api.SlotFee => ({
  rates: fee.rates.map(mapTermRate),
  slotUnitName: fee.slotUnitName,
  upgradePolicy: match(fee.upgradePolicy)
    .with(grpc.Fee_UpgradePolicy.PRORATED as 0, () => 'PRORATED' as const)
    .exhaustive(),
  downgradePolicy: match(fee.downgradePolicy)
    .with(
      grpc.Fee_DowngradePolicy.REMOVE_AT_END_OF_PERIOD as 0,
      () => 'REMOVE_AT_END_OF_PERIOD' as const
    )
    .exhaustive(),
  minimumCount: fee.minimumCount,
  quota: fee.quota,
})

const mapCapacityFee = (fee: grpc.Fee_CapacityFee): api.CapacityFee => ({
  metricId: fee.metricId,
  thresholds: fee.thresholds.map(threshold => ({
    includedAmount: threshold.includedAmount ,
    price: threshold.price,
    perUnitOverage: threshold.perUnitOverage,
  })),
})

const mapUsageFee = (fee: grpc.UsageFee): api.UsageFee => ({
  metricId: fee.metricId,
  model: mapUsageModel(fee.model),
})

const mapExtraRecurringFee = (fee: grpc.Fee_ExtraRecurringFee): api.ExtraRecurringFee => {
  let billingType: 'ARREAR' | 'ADVANCE'
  switch (fee.billingType) {
    case grpc.Fee_BillingType.ARREAR:
      billingType = 'ARREAR'
      break
    case grpc.Fee_BillingType.ADVANCE:
      billingType = 'ADVANCE'
      break
  }

  return {
    unitPrice: fee.unitPrice,
    quantity: fee.quantity,
    billingType: billingType,
    term: fee.term ? mapCadenceFromGrpc(fee.term) : undefined,
  }
}

const mapOneTimeFee = (fee: grpc.Fee_OneTimeFee): api.OneTimeFee => ({
  unitPrice: fee.unitPrice,
  quantity: fee.quantity,
})

export const mapFeeType = (feeType: grpc.Fee['feeType']): api.FeeType => {
  return match<grpc.Fee['feeType'], api.FeeType>(feeType)
    .with({ case: 'rate' }, ({ value }) => ({
      fee: 'rate',
      data: mapRateFee(value),
    }))
    .with({ case: 'slot' }, ({ value }) => ({
      fee: 'slot',
      data: mapSlotFee(value),
    }))
    .with({ case: 'capacity' }, ({ value }) => ({
      fee: 'capacity',
      data: mapCapacityFee(value),
    }))
    .with({ case: 'usage' }, ({ value }) => ({
      fee: 'usage',
      data: mapUsageFee(value),
    }))
    .with({ case: 'extraRecurring' }, ({ value }) => ({
      fee: 'extraRecurring',
      data: mapExtraRecurringFee(value),
    }))
    .with({ case: 'oneTime' }, ({ value }) => ({
      fee: 'oneTime',
      data: mapOneTimeFee(value),
    }))
    .with({ case: undefined }, () => {
      console.error('Fee type is required')
      return {
        fee: 'rate',
        data: mapRateFee(new grpc.Fee_RateFee()),
      }
    })
    .exhaustive()
}

export const mapPriceComponent = (priceComponent: grpc.PriceComponent): api.PriceComponent => {
  if (!priceComponent.fee) {
    console.error('Fee is required')
    throw new Error('Fee is required')
  }

  return {
    id: priceComponent.id,
    name: priceComponent.name,
    localId: priceComponent.localId,
    fee: mapFeeType(priceComponent.fee.feeType),
    productId: priceComponent.productId,
  }
}
