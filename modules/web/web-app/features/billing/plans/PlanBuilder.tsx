import { useMutation, createConnectQueryKey } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { ColumnDef } from '@tanstack/react-table'
import { ButtonAlt, Tabs, TabsContent, TabsList, TabsTrigger } from '@ui/components'
import { ScopeProvider } from 'jotai-scope'
import { AlertCircleIcon, ChevronLeftIcon } from 'lucide-react'
import React, { ReactNode, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { Loading } from '@/components/atoms/Loading'
import { PageSection } from '@/components/layouts/shared/PageSection'
import { SimpleTable } from '@/components/table/SimpleTable'
import { PlanActions } from '@/features/billing/plans/PlanActions'
import { PlanOverview } from '@/features/billing/plans/details/PlanDetails'
import { usePlan } from '@/features/billing/plans/hooks/usePlan'
import { PriceComponentSection } from '@/features/billing/plans/pricecomponents/PriceComponentSection'
import {
  addedComponentsAtom,
  editedComponentsAtom,
  useIsDraftVersion,
  usePlanOverview,
} from '@/features/billing/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { mapBillingPeriod, mapDate } from '@/lib/mapping'
import { listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { ListCustomerRequest_SortBy } from '@/rpc/api/customers/v1/customers_pb'
import {
  createSubscription,
  listSubscriptionsPerPlan,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { useTypedParams } from '@/utils/params'

type PriceComponentType = 'rate' | 'slot' | 'capacity' | 'usage_based' | 'scheduled'

type SidePanelOpenProps = { type: 'price-component'; data: { type: PriceComponentType } }

interface Props {
  children?: ReactNode
}
export const PlanBuilder: React.FC<Props> = ({ children }) => {
  const navigate = useNavigate()

  const { feeType } = useTypedParams<{ feeType: PriceComponentType }>()

  const [isSaving, setIsSaving] = useState(false)

  const isDraft = useIsDraftVersion()
  const overview = usePlanOverview()

  return (
    <ScopeProvider atoms={[addedComponentsAtom, editedComponentsAtom]}>
      <div className="flex h-full w-full flex-col space-y-4">
        <section className="flex justify-between pb-2 border-b border-slate-600">
          <div className="flex space-x-2 flex-row items-center">
            <ChevronLeftIcon
              className="text-2xl font-semibold cursor-pointer"
              onClick={() => navigate('..')}
            />
            <h2 className="text-2xl font-semibold">{overview?.name}</h2>
          </div>
          <div className="flex space-x-6  self-center">
            <PlanActions />
          </div>
        </section>
        {isDraft && <PlanBody />}
        {!isDraft && (
          <>
            <Tabs defaultValue="overview" className="w-full">
              <TabsList className="w-full justify-start">
                <TabsTrigger value="overview">Details</TabsTrigger>
                <TabsTrigger value="subscriptions">Subscriptions</TabsTrigger>
                <TabsTrigger value="alerts">Alerts</TabsTrigger>
              </TabsList>
              <TabsContent value="overview">
                <PlanBody />
              </TabsContent>
              <TabsContent value="subscriptions">
                <SubscriptionsTab />
              </TabsContent>
              <TabsContent value="alerts">
                <>Alerts are not implemented yet</>
              </TabsContent>
            </Tabs>
          </>
        )}
      </div>
      {children}
    </ScopeProvider>
  )
}

interface SubscriptionTableData {
  name: string
  version: number
  accrued: string
}
const SubscriptionsTab = () => {
  const overview = usePlanOverview()

  const queryClient = useQueryClient()

  const navigate = useNavigate()

  const createSubscriptionMutation = useMutation(createSubscription, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listSubscriptionsPerPlan, { planId: overview?.planId }),
      })
    },
  })

  const { data: subscriptions } = useQuery(
    listSubscriptionsPerPlan,
    {
      planId: overview?.planId!,
    },
    { enabled: !!overview }
  )

  // temporary
  const { data: customers } = useQuery(listCustomers, {
    pagination: {
      limit: 1,
      offset: 0,
    },
    sortBy: ListCustomerRequest_SortBy.NAME_ASC,
  })

  const subscriptionsData = subscriptions?.subscriptions?.map(subscription => ({
    name: subscription.customerName,
    version: subscription.version,
    accrued: '$0',
  }))

  const coustoemr = customers?.customers?.find(a => a.name === 'Comodo')

  const quickCreateSubscription = async () => {
    await createSubscriptionMutation.mutateAsync({
      planVersionId: overview?.planVersionId!,
      customerId: coustoemr?.id!,
      billingDay: 1,
      billingStart: mapDate(new Date()),
      netTerms: 0,
      parameters: {
        committedBillingPeriod: mapBillingPeriod('MONTHLY'),
        parameters: [
          {
            componentId: '3b083801-c77c-4488-848e-a185f0f0a8be',
            value: BigInt(3),
          },
        ],
      },
    })
  }
  // end temporaary

  const columns = useMemo<ColumnDef<SubscriptionTableData>[]>(
    () => [
      {
        header: 'Name',
        accessorKey: 'name',
      },
      {
        header: 'Version',
        accessorKey: 'version',
      },
      {
        header: 'Accrued',
        accessorKey: 'accrued',
      },
    ],
    []
  )

  return (
    <div>
      <div className="flex py-2 justify-end">
        <ButtonAlt type="secondary" onClick={quickCreateSubscription}>
          + New subscription
        </ButtonAlt>
      </div>

      <SimpleTable
        columns={columns}
        data={subscriptionsData ?? []}
        emptyMessage="No subscription"
      />
    </div>
  )
}

const PlanBody = () => {
  const { data: plan, isLoading } = usePlan()

  const navigate = useNavigate()

  if (isLoading) {
    return (
      <>
        <Loading />
      </>
    )
  }

  if (!plan) {
    return <>Failed to load plan</>
  }

  return (
    <>
      <PlanOverview plan={plan.planDetails?.plan!} version={plan.planDetails?.currentVersion!} />

      <PriceComponentSection />

      <PageSection
        header={{
          title: 'Trial',
          subtitle: 'Define a period during which your customers can try out this plan for free.',
        }}
      >
        <div className="py-6 space-x-4 ">
          <div className="flex items-center space-x-3 opacity-75 text-scale-1000 text-sm">
            <AlertCircleIcon size={16} strokeWidth={2} />
            <div className="text-scale-1000 w-full">This plan has no configured trial.</div>
          </div>
        </div>
      </PageSection>

      <PageSection
        header={{
          title: 'Schedules',
          subtitle: 'Define the phases of your plan.',
        }}
      >
        <div className="py-6 space-x-4 ">
          <SimpleTable columns={[]} data={[]} emptyMessage="No schedule configured" />
        </div>
      </PageSection>
      <PageSection
        header={{
          title: 'Price points',
          subtitle:
            'Define alternative prices and currencies for this plans, for specific countries or audiences.',
        }}
      >
        <Tabs defaultValue="localizations" className="w-full mt-2 py-2">
          <TabsList className="w-full justify-start">
            <TabsTrigger value="localizations">Localizations</TabsTrigger>
            <TabsTrigger value="audiences">Audiences</TabsTrigger>
            <TabsTrigger value="experimentations">Experimentations</TabsTrigger>
          </TabsList>
          <TabsContent value="localizations" className="pt-4">
            <SimpleTable
              headTrClasses="!hidden"
              columns={[]}
              data={[]}
              emptyMessage="No price point"
            />
          </TabsContent>
          <TabsContent value="audiences">Not implemented. Upvote TODO</TabsContent>
          <TabsContent value="experimentations">Not implemented. Upvote TODO</TabsContent>
        </Tabs>
      </PageSection>
      <PageSection
        header={{
          title: 'Addons',
          subtitle: 'Define the addons that can be associated with this plan',
        }}
      >
        <div className="py-6 space-x-4 ">
          <SimpleTable headTrClasses="!hidden" columns={[]} data={[]} emptyMessage="No addons" />
        </div>
      </PageSection>
    </>
  )
}
