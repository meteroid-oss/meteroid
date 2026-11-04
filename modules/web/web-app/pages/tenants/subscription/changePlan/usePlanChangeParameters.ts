import { useAtom } from 'jotai'
import { useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import {
  ComponentChoice,
  addedComponents,
  configurableComponents,
  parametersKey,
  toParameterizedComponents,
} from '@/pages/tenants/subscription/changePlan/componentChoices'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { getSubscriptionDetails } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

/** The target plan's added components to configure, and the parameters both preview and apply send. */
export const usePlanChangeParameters = () => {
  const [state, setState] = useAtom(changePlanAtom)

  const targetQuery = useQuery(
    listPriceComponents,
    { planVersionId: state.targetPlanVersionId ?? '' },
    { enabled: Boolean(state.targetPlanVersionId) }
  )
  const subscriptionQuery = useQuery(
    getSubscriptionDetails,
    { subscriptionId: state.subscriptionId },
    { enabled: Boolean(state.subscriptionId) }
  )

  const targets = targetQuery.data?.components
  const current = subscriptionQuery.data?.priceComponents
  const addedTargets = useMemo(
    () => addedComponents(targets ?? [], current ?? []),
    [targets, current]
  )
  const configurable = useMemo(
    () => configurableComponents(targets ?? [], current ?? [], state.componentChoices),
    [targets, current, state.componentChoices]
  )
  const parameterizedComponents = useMemo(
    () => toParameterizedComponents(configurable),
    [configurable]
  )

  const choose = (componentId: string, choice: ComponentChoice) =>
    setState(prev => ({
      ...prev,
      componentChoices: {
        ...prev.componentChoices,
        [componentId]: { ...prev.componentChoices[componentId], ...choice },
      },
    }))

  return {
    addedTargets,
    configurable,
    parameterizedComponents,
    parametersKey: parametersKey(parameterizedComponents),
    choose,
    // A failed lookup still lets the preview run, without parameters, to surface the server's error.
    ready: !targetQuery.isLoading && !subscriptionQuery.isLoading,
  }
}
