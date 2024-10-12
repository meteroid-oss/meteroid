import { createConnectQueryKey, disableQuery, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { atom, useAtomValue, useSetAtom } from 'jotai'
import { nanoid } from 'nanoid'
import { DeepPartial } from 'react-hook-form'
import { match } from 'ts-pattern'

import { usePlan } from '@/features/billing/plans/hooks/usePlan'
import { PriceComponent, PriceComponentType } from '@/features/billing/plans/types'
import { useQuery } from '@/lib/connectrpc'
import { BillingPeriod, mapBillingPeriod, mapBillingPeriodFromGrpc } from '@/lib/mapping'
import {
  getPlanOverviewByExternalId,
  updateDraftPlanOverview,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { useTypedParams } from '@/utils/params'

interface AddedComponent {
  ref: string
  component: DeepPartial<PriceComponent>
}
export const addedComponentsAtom = atom<AddedComponent[]>([])
export const editedComponentsAtom = atom<string[]>([])

export const useBillingPeriods = () => {
  const data = usePlanOverview()

  const queryClient = useQueryClient()

  const billingPeriods = data?.billingPeriods
    ?.map(mapBillingPeriodFromGrpc)
    .filter((period): period is BillingPeriod => !!period)
  const editDraftOverview = useMutation(updateDraftPlanOverview, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getPlanOverviewByExternalId),
      })
    },
  })

  const setBillingPeriods = (billingPeriods: BillingPeriod[]) => {
    data &&
      editDraftOverview.mutate(
        {
          ...data,
          billingPeriods: billingPeriods.map(mapBillingPeriod),
          planId: data.planId,
          planVersionId: data.planVersionId,
        },
        {}
      )
  }

  return [billingPeriods, setBillingPeriods] as const
}

export const usePlanOverview = () => {
  const { planExternalId } = useTypedParams<{ planExternalId: string }>()

  const { data } = useQuery(
    getPlanOverviewByExternalId,
    planExternalId
      ? {
          externalId: planExternalId,
        }
      : disableQuery
  )

  return data?.planOverview
}

export const useIsDraftVersion = () => {
  const plan = usePlanOverview()
  return plan?.isDraft ?? false
}

const defaults: Record<PriceComponentType, DeepPartial<PriceComponent>> = {
  rate: {
    name: 'Subscription Rate',
    // productItem: {
    //   name: 'Subscription rate',
    // },
    fee: {
      fee: 'rate',
      data: {
        rates: [],
      },
    },
  },
  slot: {
    name: 'Seats',
    fee: {
      fee: 'slot',
      data: {
        rates: [],
        downgradePolicy: 'REMOVE_AT_END_OF_PERIOD',
        upgradePolicy: 'PRORATED',
        minimumCount: 1,
        slotUnitName: 'Seats',
      },
    },
  },
  capacity: {
    name: 'Capacity commitment',
    fee: {
      fee: 'capacity',
      data: {
        thresholds: [],
      },
    },
  },
  usage: {
    name: 'Usage-based fee',
    fee: {
      fee: 'usage',
      data: {
        model: {
          model: 'per_unit',
          data: {},
        },
      },
    },
  },
  extraRecurring: {
    name: 'Recurring Charge',
    fee: {
      fee: 'extraRecurring',
      data: {
        term: 'MONTHLY',
        billingType: 'ADVANCE',
        quantity: 1,
      },
    },
  },
  oneTime: {
    name: 'One-time fee',
    fee: {
      fee: 'oneTime',
      data: {
        quantity: 1,
        unitPrice: '0',
      },
    },
  },
}

export const useAddComponent = () => {
  const setComponentsBeingCreated = useSetAtom(addedComponentsAtom)
  return (fee: PriceComponentType) => {
    const defaultValue = defaults[fee]
    const ref = nanoid()
    setComponentsBeingCreated(old => [...old, { ref, component: defaultValue }])
  }
}

export const useAddedComponents = () => {
  const added = useAtomValue(addedComponentsAtom)
  return added
}

export const useEditedComponents = () => {
  const added = useAtomValue(editedComponentsAtom)
  return added
}

export const formatPrice = (currency: string) => (price: string) => {
  const amountFloat = parseFloat(price)

  return amountFloat.toLocaleString(undefined, {
    style: 'currency',
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 8,
  })
}

export const useCurrency = () => {
  const { data: plan } = usePlan()

  return plan?.planDetails?.currentVersion?.currency ?? 'USD' // TODO
}

export const mapCadence = (cadence: 'ANNUAL' | 'QUARTERLY' | 'MONTHLY' | 'COMMITTED'): string => {
  return match(cadence)
    .with('ANNUAL', () => 'Annual')
    .with('MONTHLY', () => 'Monthly')
    .with('QUARTERLY', () => 'Quarterly')
    .with('COMMITTED', () => 'Committed')
    .exhaustive()
}

export const feeTypeToHuman = (
  type: 'rate' | 'slot' | 'capacity' | 'usage' | 'extraRecurring' | 'oneTime'
) => {
  return match(type)
    .with('rate', () => 'Subscription Rate')
    .with('slot', () => 'Slot-based')
    .with('capacity', () => 'Capacity commitment')
    .with('usage', () => 'Usage-based')
    .with('oneTime', () => 'One-time charge')
    .with('extraRecurring', () => 'Recurring charge')
    .exhaustive()
}
