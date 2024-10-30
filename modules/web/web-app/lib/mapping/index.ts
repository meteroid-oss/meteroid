import { format, parseISO } from 'date-fns'

import { BillingPeriod as BillingPeriodMessage } from '@/rpc/api/shared/v1/shared_pb'
import { Date as DateMessage } from '@/rpc/common/v1/date_pb'

export const mapDate = (date: Date): DateMessage => {
  return new DateMessage({
    year: date.getFullYear(),
    month: date.getMonth(),
    day: date.getDate(),
  })
}

export const mapDatev2 = (date: Date): string => {
  // format date to yyyy-mm-dd
  return format(date, 'yyyy-MM-dd')
}

export const mapDateFromGrpc = (date: DateMessage): Date => {
  return new Date(date.year, date.month - 1, date.day)
}

export const mapDateFromGrpcv2 = (date: string): Date => {
  return parseISO(date)
}

export type BillingPeriod = 'MONTHLY' | 'QUARTERLY' | 'ANNUAL'
export const mapBillingPeriod = (period: BillingPeriod): BillingPeriodMessage => {
  switch (period) {
    case 'MONTHLY':
      return BillingPeriodMessage.MONTHLY
    case 'QUARTERLY':
      return BillingPeriodMessage.QUARTERLY
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
    case BillingPeriodMessage.ANNUAL:
      return 'ANNUAL'
  }
}

export const sortBillingPeriods = (periods: BillingPeriod[]) => {
  return periods.sort((a, b) => {
    const order = ['MONTHLY', 'QUARTERLY', 'ANNUAL']
    return order.indexOf(a) - order.indexOf(b)
  })
}
