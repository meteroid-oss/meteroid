import { format, parseISO } from 'date-fns'

import { BillingPeriod as BillingPeriodMessage } from '@/rpc/api/shared/v1/shared_pb'

// Dates are exchanged with the backend as ISO strings (yyyy-MM-dd for plain dates).
export const mapDate = (date: Date): string => {
  // format date to yyyy-mm-dd
  return format(date, 'yyyy-MM-dd')
}

export const mapDateFromGrpc = (date: string): Date => {
  return parseISO(date)
}

export type BillingPeriod = 'MONTHLY' | 'QUARTERLY' | 'SEMIANNUAL' | 'ANNUAL'
export const mapBillingPeriod = (period: BillingPeriod): BillingPeriodMessage => {
  switch (period) {
    case 'MONTHLY':
      return BillingPeriodMessage.MONTHLY
    case 'QUARTERLY':
      return BillingPeriodMessage.QUARTERLY
    case 'SEMIANNUAL':
      return BillingPeriodMessage.SEMIANNUAL
    case 'ANNUAL':
      return BillingPeriodMessage.ANNUAL
  }
}

export const mapBillingPeriodFromGrpc = (period: BillingPeriodMessage): BillingPeriod => {
  switch (period) {
    case BillingPeriodMessage.MONTHLY:
      return 'MONTHLY'
    case BillingPeriodMessage.QUARTERLY:
      return 'QUARTERLY'
    case BillingPeriodMessage.SEMIANNUAL:
      return 'SEMIANNUAL'
    case BillingPeriodMessage.ANNUAL:
      return 'ANNUAL'
  }
}

export const sortBillingPeriods = (periods: BillingPeriod[]) => {
  return periods.sort((a, b) => {
    const order = ['MONTHLY', 'QUARTERLY', 'SEMIANNUAL', 'ANNUAL']
    return order.indexOf(a) - order.indexOf(b)
  })
}
