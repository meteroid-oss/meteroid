import { Skeleton } from '@md/ui'
import { useAtom } from 'jotai'
import { useResetAtom } from 'jotai/utils'
import { Fragment, useEffect } from 'react'
import { Wizard } from 'react-use-wizard'

import PageHeading from '@/components/PageHeading/PageHeading'
import { useQuery } from '@/lib/connectrpc'
import { StepConfirm } from '@/pages/tenants/subscription/changePlan/StepConfirm'
import { StepReviewMapping } from '@/pages/tenants/subscription/changePlan/StepReviewMapping'
import { StepSelectPlan } from '@/pages/tenants/subscription/changePlan/StepSelectPlan'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import { getSubscriptionDetails } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { useTypedParams } from '@/utils/params'

export const ChangePlanWizard = () => {
  const resetState = useResetAtom(changePlanAtom)
  const [, setState] = useAtom(changePlanAtom)
  const { subscriptionId } = useTypedParams()

  const subscriptionQuery = useQuery(
    getSubscriptionDetails,
    { subscriptionId: subscriptionId ?? '' },
    { enabled: Boolean(subscriptionId) }
  )

  const subscription = subscriptionQuery.data?.subscription

  useEffect(() => {
    if (subscription) {
      setState(prev => ({
        ...prev,
        subscriptionId: subscription.id,
        currentPlanVersionId: subscription.planVersionId,
        currentPlanName: subscription.planName,
        currency: subscription.currency,
      }))
    }
  }, [subscription?.id])

  useEffect(() => {
    return () => {
      resetState()
    }
  }, [])

  if (subscriptionQuery.isLoading || !subscription) {
    return (
      <div className="p-6">
        <Skeleton height={16} width={50} className="mb-4" />
        <Skeleton height={200} className="mb-4" />
      </div>
    )
  }

  return (
    <Fragment>
      <PageHeading>Change Plan</PageHeading>
      <div className="flex flex-col pt-8">
        <Wizard>
          <StepSelectPlan />
          <StepReviewMapping />
          <StepConfirm />
        </Wizard>
      </div>
    </Fragment>
  )
}
