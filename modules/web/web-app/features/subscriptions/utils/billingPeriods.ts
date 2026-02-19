import { getBillingPeriodLabel, getPrice, getPriceBillingLabel } from '@/lib/mapping/priceToSubscriptionFee'
import { PriceComponent as GrpcPriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

/**
 * Get billing period label for a gRPC PriceComponent.
 * Handles configuration override (e.g. user selected a different billing period).
 */
export const getApiComponentBillingPeriodLabel = (
  component: GrpcPriceComponent,
  configuration?: { billingPeriod?: BillingPeriod }
): string => {
  if (configuration?.billingPeriod !== undefined) {
    return getBillingPeriodLabel(configuration.billingPeriod)
  }

  const price = getPrice(component)
  if (price) {
    return getPriceBillingLabel(price)
  }

  return 'Monthly'
}
