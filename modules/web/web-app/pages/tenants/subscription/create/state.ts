import { atom } from 'jotai'

export const createSubscriptionAtom = atom<{
  customerId?: string
  planExternalId?: string
  fromDate: Date
  toDate?: Date
  billingDay: 'FIRST' | 'SUB_START_DAY'
}>({
  customerId: undefined,
  planExternalId: undefined,
  fromDate: new Date(),
  toDate: undefined,
  billingDay: 'FIRST',
})
