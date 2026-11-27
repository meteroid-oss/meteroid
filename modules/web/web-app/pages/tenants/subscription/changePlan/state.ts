import { atomWithReset } from 'jotai/utils'

import { ComponentChoice } from '@/pages/tenants/subscription/changePlan/componentChoices'
import {
  PlanChangeApplyMode,
  PreviewPlanChangeResponse,
} from '@/rpc/api/subscriptions/v1/subscriptions_pb'

export interface ChangePlanState {
  subscriptionId: string
  currentPlanVersionId: string
  currentPlanName: string
  currency: string
  /** The alias of the plan option the subscription bought, when it still exists. */
  currentOption?: string
  targetPlanVersionId?: string
  targetPlanName?: string
  /** The alias of the target version's option bought; unset when it has none. */
  option?: string
  preview?: PreviewPlanChangeResponse
  applyMode: PlanChangeApplyMode
  componentChoices: Record<string, ComponentChoice>
  /** Prices set on the target's charges priced by one amount, by component; empty: list. */
  prices: Record<string, string>
}

export const changePlanAtom = atomWithReset<ChangePlanState>({
  subscriptionId: '',
  currentPlanVersionId: '',
  currentPlanName: '',
  currency: '',
  currentOption: undefined,
  targetPlanVersionId: undefined,
  targetPlanName: undefined,
  option: undefined,
  preview: undefined,
  applyMode: PlanChangeApplyMode.END_OF_PERIOD,
  componentChoices: {},
  prices: {},
})
