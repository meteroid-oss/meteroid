import { atomWithReset } from 'jotai/utils'

import { PreviewPlanChangeResponse } from '@/rpc/api/subscriptions/v1/subscriptions_pb'

export interface ChangePlanState {
  subscriptionId: string
  currentPlanVersionId: string
  currentPlanName: string
  currency: string
  targetPlanVersionId?: string
  targetPlanName?: string
  preview?: PreviewPlanChangeResponse
}

export const changePlanAtom = atomWithReset<ChangePlanState>({
  subscriptionId: '',
  currentPlanVersionId: '',
  currentPlanName: '',
  currency: '',
  targetPlanVersionId: undefined,
  targetPlanName: undefined,
  preview: undefined,
})
