import { disableQuery } from '@connectrpc/connect-query'
import { Button, Tabs, TabsContent, TabsList, TabsTrigger } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { ScopeProvider } from 'jotai-scope'
import { ChevronLeftIcon } from 'lucide-react'
import { ReactNode, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { Loading } from '@/components/Loading'
import { PageSection } from '@/components/layouts/shared/PageSection'
import { SimpleTable } from '@/components/table/SimpleTable'
import { ListPlanVersionTab } from '@/features/billing/plans/ListPlanVersion'
import { PlanActions } from '@/features/billing/plans/PlanActions'
import { PlanOverview } from '@/features/billing/plans/details/PlanDetails'
import {
  useIsDraftVersion,
  usePlanOverview,
  usePlanWithVersion,
} from '@/features/billing/plans/hooks/usePlan'
import { PriceComponentSection } from '@/features/billing/plans/pricecomponents/PriceComponentSection'
import {
  addedComponentsAtom,
  editedComponentsAtom,
} from '@/features/billing/plans/pricecomponents/utils'
import { PlanTrial } from '@/features/billing/plans/trial/PlanTrial'
import { SubscriptionsTable } from '@/features/subscriptions'
import { useQuery } from '@/lib/connectrpc'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

interface Props {
  children?: ReactNode
}

export const PlanBuilder: React.FC<Props> = ({ children }) => {
  const navigate = useNavigate()

  const isDraft = useIsDraftVersion()
  const overview = usePlanOverview()

  return (
    <ScopeProvider atoms={[addedComponentsAtom, editedComponentsAtom]}>
      <div className="flex h-full w-full flex-col space-y-4">
        <section className="flex justify-between pb-2 border-b border-border">
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
                <TabsTrigger value="versions">History</TabsTrigger>
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
              <TabsContent value="versions">
                <ListPlanVersionTab />
              </TabsContent>
            </Tabs>
          </>
        )}
      </div>
      {children}
    </ScopeProvider>
  )
}

const SubscriptionsTab = () => {
  const overview = usePlanOverview()

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 15,
  })

  const subscriptionsQuery = useQuery(
    listSubscriptions,
    overview
      ? {
          planId: overview.id,
          pagination: {
            perPage: pagination.pageSize,
            page: pagination.pageIndex,
          },
        }
      : disableQuery
  )

  const data = subscriptionsQuery.data?.subscriptions ?? []
  const count = Number(subscriptionsQuery.data?.pagination?.totalItems ?? 0)
  const isLoading = subscriptionsQuery.isLoading

  return (
    <div>
      <div className="flex py-2 justify-end">
        <Button variant="secondary" onClick={() => toast('Unimplemented')}>
          + New subscription
        </Button>
      </div>

      <SubscriptionsTable
        data={data}
        totalCount={count}
        pagination={pagination}
        setPagination={setPagination}
        isLoading={isLoading}
        hidePlan
      />
    </div>
  )
}

const PlanBody = () => {
  const planData = usePlanWithVersion()

  if (planData.isLoading) {
    return (
      <>
        <Loading />
      </>
    )
  }

  if (!planData?.plan || !planData.version) {
    return <>Failed to load plan</>
  }

  const plan = planData.plan
  const current = planData.version

  return (
    <>
      {current && <PlanOverview plan={plan} version={current} />}
      {plan.planType !== PlanType.FREE && (
        <>
          <PriceComponentSection />

          <PageSection
            header={{
              title: 'Trial',
              subtitle:
                'Define a period during which your customers can try out this plan for free.',
            }}
          >
            <PlanTrial
              config={current?.trialConfig}
              currentPlanId={plan.id}
              currentPlanVersionId={current.id}
              currentPlanLocalId={plan.localId}
            />
          </PageSection>

          <PageSection
            header={{
              title: 'Schedules',
              subtitle: 'Define the phases of your plan.',
            }}
          >
            <div className="space-x-4 ">
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
            <Tabs defaultValue="localizations" className="w-full">
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
        </>
      )}

      <PageSection
        header={{
          title: 'Addons',
          subtitle: 'Define the addons that can be associated with this plan',
        }}
      >
        <div className="space-x-4 ">
          <SimpleTable headTrClasses="!hidden" columns={[]} data={[]} emptyMessage="No addons" />
        </div>
      </PageSection>
    </>
  )
}
