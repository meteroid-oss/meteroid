import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useSetAtom, atom, useAtomValue } from 'jotai'
import { nanoid } from 'nanoid'
import { DeepPartial } from 'react-hook-form'
import { match } from 'ts-pattern'

import { usePlan } from '@/features/billing/plans/hooks/usePlan'
import { PriceComponent, PriceComponentType } from '@/features/billing/plans/types'
import { useQuery } from '@/lib/connectrpc'
import { mapBillingPeriodFromGrpc, BillingPeriod, mapBillingPeriod } from '@/lib/mapping'
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

  const { data } = useQuery(getPlanOverviewByExternalId, {
    externalId: planExternalId!,
  })

  return data?.planOverview
}

export const useIsDraftVersion = () => {
  const plan = usePlanOverview()
  return plan?.isDraft ?? false
}

const defaults: Record<PriceComponentType, DeepPartial<PriceComponent>> = {
  rate: {
    name: 'Subscription Rate',
    productItem: {
      name: 'Subscription rate',
    },
    fee: {
      fee: 'rate',
      data: {
        pricing: {
          rates: [],
          cadence: 'COMMITTED',
        },
      },
    },
  },
  slot_based: {
    name: 'Seats',
    productItem: {
      name: 'Subscription rate',
    },
    fee: {
      fee: 'slot_based',
      data: {
        downgradePolicy: 'REMOVE_AT_END_OF_PERIOD',
        upgradePolicy: 'PRORATED',
        minimumCount: 1,
        slotUnit: {
          name: 'Seats',
        },
        pricing: {
          rates: [],
          cadence: 'COMMITTED',
        },
      },
    },
  },
  capacity: {
    name: 'Capacity commitment',
    productItem: {
      name: 'Usage fees',
    },
    fee: {
      fee: 'capacity',
      data: {
        metric: {},
        pricing: {
          thresholds: [],
          cadence: 'COMMITTED',
        },
      },
    },
  },
  usage_based: {
    name: 'Usage-based fee',
    productItem: {
      name: 'Usage fees',
    },
    fee: {
      fee: 'usage_based',
      data: {
        metric: {},
        model: {
          model: 'per_unit',
          data: {},
        },
      },
    },
  },
  recurring: {
    name: 'Recurring Charge',
    productItem: {
      name: 'Fixed charge',
    },
    fee: {
      fee: 'recurring',
      data: {
        cadence: 'MONTHLY',
        fee: {
          billingType: 'ADVANCE',
          quantity: 1,
        },
      },
    },
  },
  one_time: {
    name: 'One-time fee',
    productItem: {
      name: 'Fixed charge',
    },
    fee: {
      fee: 'one_time',
      data: {
        pricing: {
          billingType: 'ADVANCE',
          quantity: 1,
        },
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
  type: 'rate' | 'slot_based' | 'capacity' | 'usage_based' | 'recurring' | 'one_time'
) => {
  return match(type)
    .with('rate', () => 'Subscription Rate')
    .with('slot_based', () => 'Slot-based')
    .with('capacity', () => 'Capacity commitment')
    .with('usage_based', () => 'Usage-based')
    .with('one_time', () => 'One-time charge')
    .with('recurring', () => 'Recurring charge')
    .exhaustive()
}
