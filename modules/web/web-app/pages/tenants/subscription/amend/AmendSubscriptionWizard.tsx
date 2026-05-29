import { Skeleton } from '@md/ui'
import { useAtom } from 'jotai'
import { useResetAtom } from 'jotai/utils'
import { Fragment, useEffect } from 'react'
import { Wizard } from 'react-use-wizard'

import PageHeading from '@/components/PageHeading/PageHeading'
import { useQuery } from '@/lib/connectrpc'
import { StepEditChanges } from '@/pages/tenants/subscription/amend/StepEditChanges'
import { StepReviewApply } from '@/pages/tenants/subscription/amend/StepReviewApply'
import { amendSubscriptionAtom } from '@/pages/tenants/subscription/amend/state'
import { getSubscriptionDetails } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { useTypedParams } from '@/utils/params'

export const AmendSubscriptionWizard = () => {
  const resetState = useResetAtom(amendSubscriptionAtom)
  const [, setState] = useAtom(amendSubscriptionAtom)
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
      <PageHeading>Amend Subscription</PageHeading>
      <div className="flex flex-col pt-8">
        <Wizard>
          <StepEditChanges />
          <StepReviewApply />
        </Wizard>
      </div>
    </Fragment>
  )
}
