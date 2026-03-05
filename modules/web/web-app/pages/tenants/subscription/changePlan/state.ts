import { atomWithReset } from 'jotai/utils'

import {
  PlanChangeApplyMode,
  PreviewPlanChangeResponse,
} from '@/rpc/api/subscriptions/v1/subscriptions_pb'

export interface ChangePlanState {
  subscriptionId: string
  currentPlanVersionId: string
  currentPlanName: string
  currency: string
  targetPlanVersionId?: string
  targetPlanName?: string
  preview?: PreviewPlanChangeResponse
  applyMode: PlanChangeApplyMode
}

export const changePlanAtom = atomWithReset<ChangePlanState>({
  subscriptionId: '',
  currentPlanVersionId: '',
  currentPlanName: '',
  currency: '',
  targetPlanVersionId: undefined,
  targetPlanName: undefined,
  preview: undefined,
  applyMode: PlanChangeApplyMode.END_OF_PERIOD,
})
