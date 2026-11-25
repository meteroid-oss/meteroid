import { useAtom } from 'jotai'
import { useMemo } from 'react'

import { findOption, inOption } from '@/features/subscriptions/pricecomponents/planOption'
import { useQuery } from '@/lib/connectrpc'
import {
  ComponentChoice,
  addedComponents,
  configureAdded,
  configureMatched,
  matchedComponents,
  parametersKey,
  toParameterizedComponents,
} from '@/pages/tenants/subscription/changePlan/componentChoices'
import { changePlanAtom } from '@/pages/tenants/subscription/changePlan/state'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
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
  const versionQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: state.targetPlanVersionId ?? '' },
    { enabled: Boolean(state.targetPlanVersionId) }
  )
  const subscriptionQuery = useQuery(
    getSubscriptionDetails,
    { subscriptionId: state.subscriptionId },
    { enabled: Boolean(state.subscriptionId) }
  )

  const targets = targetQuery.data?.components
  const options = versionQuery.data?.plan?.version?.options
  const current = subscriptionQuery.data?.priceComponents
  // Matched by product on the whole charge, then shown and configured in the option's cell.
  const addedTargets = useMemo(
    () =>
      inOption(
        addedComponents(targets ?? [], current ?? []),
        findOption(options ?? [], state.option),
        state.currency
      ),
    [targets, current, options, state.option, state.currency]
  )
  const configurable = useMemo(
    () => configureAdded(addedTargets, state.componentChoices),
    [addedTargets, state.componentChoices]
  )
  // A held usage-levels charge the target sells at several levels: its level, always sent, as the
  // server finds none when the target lacks the one held.
  const matchedConfigurable = useMemo(
    () =>
      configureMatched(
        inOption(
          matchedComponents(targets ?? [], current ?? []),
          findOption(options ?? [], state.option),
          state.currency
        ),
        current ?? [],
        state.componentChoices
      ),
    [targets, current, options, state.option, state.currency, state.componentChoices]
  )
  const parameterizedComponents = useMemo(
    () => toParameterizedComponents([...configurable, ...matchedConfigurable]),
    [configurable, matchedConfigurable]
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
    matchedConfigurable,
    parameterizedComponents,
    // The option is sent with the parameters: a change of option re-runs the preview.
    option: state.option,
    parametersKey: `${state.option ?? ''}|${parametersKey(parameterizedComponents)}`,
    choose,
    // A failed lookup still lets the preview run, without parameters, to surface the server's error.
    ready: !targetQuery.isLoading && !versionQuery.isLoading && !subscriptionQuery.isLoading,
  }
}
