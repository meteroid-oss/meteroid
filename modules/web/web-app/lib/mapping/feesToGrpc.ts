import { PlainMessage } from '@bufbuild/protobuf'
import { match } from 'ts-pattern'

import * as grpc from '@/rpc/api/pricecomponents/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import * as api from '../schemas/plans'

export const mapCadence = (cadence: api.Cadence): BillingPeriod => {
  return match(cadence)
    .with('MONTHLY', () => BillingPeriod.MONTHLY)
    .with('QUARTERLY', () => BillingPeriod.QUARTERLY)
    .with('ANNUAL', () => BillingPeriod.ANNUAL)
    .exhaustive()
}

const mapRateFee = (fee: api.RateFee): grpc.Fee_RateFee => {
  const data: PlainMessage<grpc.Fee_RateFee> = {
    rates: fee.rates.map(rate => ({
      term: mapCadence(rate.term),
      price: rate.price,
    })),
  }

  return new grpc.Fee_RateFee(data)
}

const mapSlotFee = (fee: api.SlotFee): grpc.Fee_SlotFee => {
  const data: PlainMessage<grpc.Fee_SlotFee> = {
    rates: fee.rates.map(rate => ({
      term: mapCadence(rate.term),
      price: rate.price,
    })),
    slotUnitName: fee.slotUnitName,
    upgradePolicy: grpc.Fee_UpgradePolicy[fee.upgradePolicy],
    downgradePolicy: grpc.Fee_DowngradePolicy[fee.downgradePolicy],
    minimumCount: fee.minimumCount,
    quota: fee.quota,
  }

  return new grpc.Fee_SlotFee(data)
}

const mapCapacityFee = (fee: api.CapacityFee): grpc.Fee_CapacityFee => {
  const data: PlainMessage<grpc.Fee_CapacityFee> = {
    metricId: fee.metricId,
    thresholds: fee.thresholds.map(threshold => ({
      includedAmount: BigInt(threshold.includedAmount),
      price: threshold.price,
      perUnitOverage: threshold.perUnitOverage,
    })),
  }

  return new grpc.Fee_CapacityFee(data)
}

const mapUsageFee = (fee: api.UsageFee): grpc.UsageFee => {
  let model: grpc.UsageFee['model']

  switch (fee.model.model) {
    case 'per_unit':
      model = {
        case: 'perUnit',
        value: fee.model.data.unitPrice,
      }
      break
    case 'package': {
      const data: PlainMessage<grpc.UsageFee_Package> = {
        blockSize: BigInt(fee.model.data.blockSize),
        packagePrice: fee.model.data.packagePrice,
      }
      model = {
        case: 'package',
        value: new grpc.UsageFee_Package(data),
      }
      break
    }
    case 'matrix': {
      const rows: PlainMessage<grpc.UsageFee_Matrix_MatrixRow>[] =
        fee.model.data.dimensionRates.map(rate => ({
          dimension1: rate.dimension1,
          dimension2: rate.dimension2,
          perUnitPrice: rate.price,
        }))

      const data: PlainMessage<grpc.UsageFee_Matrix> = {
        rows: rows.map(row => new grpc.UsageFee_Matrix_MatrixRow(row)),
      }

      model = {
        case: fee.model.model,
        value: new grpc.UsageFee_Matrix(data),
      }
      break
    }
    case 'tiered':
    case 'volume': {
      const rows: PlainMessage<grpc.UsageFee_TieredAndVolume_TierRow>[] = fee.model.data.rows.map(
        tier => ({
          firstUnit: BigInt(tier.firstUnit),
          unitPrice: tier.unitPrice,
          flatFee: tier.flatFee,
          flatCap: tier.flatCap,
        })
      )

      const blockSize = fee.model.data.blockSize ? BigInt(fee.model.data.blockSize) : undefined

      const data: PlainMessage<grpc.UsageFee_TieredAndVolume> = {
        rows: rows.map(row => new grpc.UsageFee_TieredAndVolume_TierRow(row)),
        blockSize,
      }

      model = {
        case: fee.model.model,
        value: new grpc.UsageFee_TieredAndVolume(data),
      }
      break
    }
  }

  const data: PlainMessage<grpc.UsageFee> = {
    metricId: fee.metricId,
    model: model,
  }

  return new grpc.UsageFee(data)
}

const mapExtraRecurringFee = (fee: api.ExtraRecurringFee): grpc.Fee_ExtraRecurringFee => {
  const data: PlainMessage<grpc.Fee_ExtraRecurringFee> = {
    unitPrice: fee.unitPrice,
    quantity: fee.quantity,
    billingType: grpc.Fee_BillingType[fee.billingType],
    term: fee.term ? mapCadence(fee.term) : undefined,
  }

  return new grpc.Fee_ExtraRecurringFee(data)
}

const mapOneTimeFee = (fee: api.OneTimeFee): grpc.Fee_OneTimeFee => {
  const data: PlainMessage<grpc.Fee_OneTimeFee> = {
    unitPrice: fee.unitPrice,
    quantity: fee.quantity,
  }

  return new grpc.Fee_OneTimeFee(data)
}

export const mapFee = (feeType: api.FeeType): grpc.Fee => {
  const mappedFeeType: grpc.Fee['feeType'] = match<api.FeeType, grpc.Fee['feeType']>(feeType)
    .with({ fee: 'rate' }, ({ data }) => ({
      case: 'rate',
      value: mapRateFee(data),
    }))
    .with({ fee: 'slot' }, ({ data }) => ({
      case: 'slot',
      value: mapSlotFee(data),
    }))
    .with({ fee: 'capacity' }, ({ data }) => ({
      case: 'capacity',
      value: mapCapacityFee(data),
    }))
    .with({ fee: 'usage' }, ({ data }) => ({
      case: 'usage',
      value: mapUsageFee(data),
    }))
    .with({ fee: 'extraRecurring' }, ({ data }) => ({
      case: 'extraRecurring',
      value: mapExtraRecurringFee(data),
    }))
    .with({ fee: 'oneTime' }, ({ data }) => ({
      case: 'oneTime',
      value: mapOneTimeFee(data),
    }))
    .exhaustive()

  const fee: PlainMessage<grpc.Fee> = {
    feeType: mappedFeeType,
  }

  return new grpc.Fee(fee)
}

export const mapPriceComponent = (
  priceComponent: api.PriceComponent
): PlainMessage<grpc.PriceComponent> => {
  return {
    id: priceComponent.id,
    name: priceComponent.name,
    fee: mapFee(priceComponent.fee),
    productItemId: priceComponent.productItemId,
  }
}
