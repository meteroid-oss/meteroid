import { PriceComponent } from '@/lib/schemas/plans'
import { PriceComponent as GrpcPriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import { BillingPeriod, BillingPeriod as SharedBillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

export const mapTermToBillingPeriod = (term: string): SharedBillingPeriod => {
  switch (term) {
    case 'MONTHLY':
      return SharedBillingPeriod.MONTHLY
    case 'QUARTERLY':
      return SharedBillingPeriod.QUARTERLY
    case 'ANNUAL':
      return SharedBillingPeriod.ANNUAL
    default:
      return SharedBillingPeriod.MONTHLY
  }
}

export const getBillingPeriodLabel = (period: SharedBillingPeriod): string => {
  switch (period) {
    case SharedBillingPeriod.MONTHLY:
      return 'Monthly'
    case SharedBillingPeriod.QUARTERLY:
      return 'Quarterly'
    case SharedBillingPeriod.ANNUAL:
      return 'Annual'
    default:
      return 'Monthly'
  }
}



// For schema-based components (CreateSubscriptionPriceComponents)
export const getSchemaComponentBillingPeriodLabel = (
  component: PriceComponent, 
  configuration?: { billingPeriod?: SharedBillingPeriod }
): string => {
  const feeType = component.fee.fee
  
  // For usage & capacity: always monthly
  if (feeType === 'usage' || feeType === 'capacity') {
    return 'Monthly'
  }
  
  // For rates and slots: use configured period or the only available rate's term
  if (feeType === 'rate' || feeType === 'slot') {
    if (configuration?.billingPeriod !== undefined) {
      return getBillingPeriodLabel(configuration.billingPeriod)
    } else {
      // Use the first (or only) rate's term
      const rates = component.fee.data.rates
      if (rates && rates.length > 0) {
        switch (rates[0].term) {
          case 'MONTHLY':
            return 'Monthly'
          case 'QUARTERLY':
            return 'Quarterly'
          case 'ANNUAL':
            return 'Annual'
          default:
            return 'Monthly'
        }
      }
    }
  }
  
  // For one-time and extra recurring
  if (feeType === 'oneTime') {
    return 'One-time'
  }
  if (feeType === 'extraRecurring') {
    return 'Monthly'
  }
  
  return 'Monthly'
}

// For API response components (StepReviewAndCreate)
export const getApiComponentBillingPeriodLabel = (
  component: GrpcPriceComponent, 
  configuration?: { billingPeriod?: BillingPeriod }
): string => {
  const feeType = component.fee?.feeType?.case
  
  // For usage & capacity: always monthly
  if (feeType === 'usage' || feeType === 'capacity') {
    return 'Monthly'
  }
  
  // For rates and slots: use configured period or the only available rate's term
  if (feeType === 'rate' || feeType === 'slot') {
    if (configuration?.billingPeriod !== undefined) {
      switch (configuration.billingPeriod) {
        case BillingPeriod.MONTHLY:  
          return 'Monthly'
        case  BillingPeriod.QUARTERLY: 
          return 'Quarterly'
        case BillingPeriod.ANNUAL:  
          return 'Annual'
        default:
          return 'Monthly'
      }
    } else {
      // Use the first (or only) rate's term from API response
      if (component.fee?.feeType?.case === 'rate' && component.fee.feeType.value) {
        const rateFee = component.fee.feeType.value
        if ('rates' in rateFee && rateFee.rates && rateFee.rates.length > 0) {
          const firstRate = rateFee.rates[0]
          if ('term' in firstRate) {
            return getBillingPeriodLabel(firstRate.term)

          }
        }
      } else if (component.fee?.feeType?.case === 'slot' && component.fee.feeType.value) {
        const slotFee = component.fee.feeType.value
        if ('rates' in slotFee && slotFee.rates && slotFee.rates.length > 0) {
          const firstRate = slotFee.rates[0]
          if ('term' in firstRate) {
            return getBillingPeriodLabel(firstRate.term)
          }
        }
      }
    }
  }
  
  // For one-time and extra recurring
  if (feeType === 'oneTime') {
    return 'One-time'
  }
  if (feeType === 'extraRecurring') {
    return 'Monthly'
  }
  
  return 'Monthly'
}
 




export const getSubscriptionComponentBillingPeriodLabel = (feeType?: string): string => {
  switch (feeType) {
    case 'oneTime':
      return 'One-time'
    case 'extraRecurring':
    case 'rate':
      return 'Monthly'
    case 'slot':
      return 'Monthly' // Default for slots in extra components
    case 'capacity':
      return 'Monthly'
    default:
      return 'One-time'
  }
}

// Keep the old export for backward compatibility
export const getExtraComponentBillingPeriodLabel = getSubscriptionComponentBillingPeriodLabel