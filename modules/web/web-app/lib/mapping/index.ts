import { BillingPeriod as BillingPeriodMessage } from '@/rpc/api/shared/v1/shared_pb'
import { Date as DateMessage } from '@/rpc/common/v1/date_pb'

export const mapDate = (date: Date): DateMessage => {
  return new DateMessage({
    year: date.getFullYear(),
    month: date.getMonth(),
    day: date.getDate(),
  })
}

export const mapDateFromGrpc = (date: DateMessage): Date => {
  return new Date(date.year, date.month, date.day)
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
