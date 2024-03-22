import { atom } from 'jotai'

export const createSubscriptionAtom = atom<{
  customerId?: string
  planVersionId?: string
  fromDate: Date
  toDate?: Date
  billingDay: 'FIRST' | 'SUB_START_DAY'
}>({
  customerId: undefined,
  planVersionId: undefined,
  fromDate: new Date(),
  toDate: undefined,
  billingDay: 'FIRST',
})
