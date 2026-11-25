import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

export function formatCadence(cadence: BillingPeriod): string {
  switch (cadence) {
    case BillingPeriod.MONTHLY:
      return 'Monthly'
    case BillingPeriod.QUARTERLY:
      return 'Quarterly'
    case BillingPeriod.SEMIANNUAL:
      return 'Semiannual'
    case BillingPeriod.ANNUAL:
      return 'Annual'
    default:
      return 'Unknown'
  }
}
