import { disableQuery } from '@connectrpc/connect-query'
import {
  Button,
  Form,
  GenericFormField,
} from '@ui/components'
import { useAtom } from 'jotai'
import { useEffect, useRef, useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { z } from 'zod'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { AddOnCouponSelector } from '@/features/addons/AddOnCouponSelector'
import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { SubscribablePlanVersionSelect } from '@/features/plans/SubscribablePlanVersionSelect'
import { CreateSubscriptionPriceComponents } from '@/features/subscriptions/pricecomponents/CreateSubscriptionPriceComponents'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

// TODO confirm & reset form on leave
export const StepPlanAndCustomer = () => {
  const { nextStep } = useWizard()
  const [state, setState] = useAtom(createSubscriptionAtom)
  const [searchParams] = useSearchParams()
  const [isValid, setIsValid] = useState(true)
  const [validationErrors, setValidationErrors] = useState<string[]>([])

  const customerIdFromUrl = searchParams.get('customerId')
  const planVersionIdFromUrl = searchParams.get('planVersionId')

  const methods = useZodForm({
    schema: schema,
    defaultValues: {
      customerId: state.customerId || customerIdFromUrl || undefined,
      planVersionId: state.planVersionId || planVersionIdFromUrl || undefined,
    },
  })

  const [customerId, planVersionId] = methods.watch(['customerId', 'planVersionId'])

  // Reset component configs when switching plans to avoid stale component IDs
  const prevPlanVersionId = useRef(planVersionId)
  useEffect(() => {
    if (prevPlanVersionId.current && planVersionId !== prevPlanVersionId.current) {
      setState(prev => ({
        ...prev,
        components: { parameterized: [], overridden: [], extra: [], removed: [] },
      }))
    }
    prevPlanVersionId.current = planVersionId
  }, [planVersionId, setState])

  const addOnsQuery = useQuery(
    listAddOns,
    planVersionId
      ? {
          planVersionId,
          pagination: {
            perPage: 100,
            page: 0,
          },
        }
      : disableQuery
  )
  const couponsQuery = useQuery(listCoupons, {
    pagination: {
      perPage: 100,
      page: 0,
    },
    filter: ListCouponRequest_CouponFilter.ACTIVE,
  })

  const planQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: planVersionId! },
    { enabled: !!planVersionId }
  )

  const selectedPlanId = planQuery.data?.plan?.plan?.id

  const availableAddOns = addOnsQuery.data?.addOns || []
  const availableCoupons = couponsQuery.data?.coupons || []

  const isCouponAvailableForPlan = (coupon: (typeof availableCoupons)[0]) => {
    if (!coupon.planIds || coupon.planIds.length === 0) return true
    if (!selectedPlanId) return true
    return coupon.planIds.includes(selectedPlanId)
  }

  const onSubmit = async (data: z.infer<typeof schema>) => {
    setState({
      ...state,
      ...data,
    })
    nextStep()
  }

  // console.log([customerId, planVersionId])

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <PageSection
          className="fadeIn"
          header={{
            title: 'Plan & Customer',
            subtitle: 'Choose the owner of the subscription',
          }}
        >
          <div className="flex flex-col gap-4 max-w-xl">
            <GenericFormField
              control={methods.control}
              layout="horizontal"
              label="Plan"
              name="planVersionId"
              render={({ field }) => (
                <SubscribablePlanVersionSelect value={field.value} onChange={field.onChange} />
              )}
            />
            <GenericFormField
              control={methods.control}
              layout="horizontal"
              label="Customer"
              name="customerId"
              render={({ field }) => (
                <CustomerSelect value={field.value} onChange={field.onChange} />
              )}
            />
          </div>
        </PageSection>
        {planVersionId && customerId && (
          <>
            <PageSection
              className="fadeIn"
              header={{
                title: 'Pricing',
                subtitle: 'Configure the price components attached to this subscription',
                actions: null,
              }}
            >
              {/* <div>
                <Alert variant="destructive">
                  <AlertDescription>
                    !! WIP !! This UI only works for non-parametrized price component in this
                    iteration
                  </AlertDescription>
                </Alert>
              </div> */}
              {planQuery.data?.plan?.version?.currency && (
                <CreateSubscriptionPriceComponents
                  planVersionId={planVersionId}
                  currency={planQuery.data.plan.version.currency}
                  onValidationChange={(valid, errors) => {
                    setIsValid(valid)
                    setValidationErrors(errors)
                  }}
                />
              )}
            </PageSection>

            <PageSection
              className="fadeIn"
              header={{
                title: 'Add-ons & Discounts',
                subtitle: 'Optional enhancements and promotional offers',
              }}
            >
              <AddOnCouponSelector
                selectedAddOns={state.addOns}
                onAddOnAdd={id =>
                  setState(prev => ({ ...prev, addOns: [...prev.addOns, { addOnId: id }] }))
                }
                onAddOnRemove={id =>
                  setState(prev => ({
                    ...prev,
                    addOns: prev.addOns.filter(a => a.addOnId !== id),
                  }))
                }
                availableAddOns={availableAddOns}
                selectedCoupons={state.coupons}
                onCouponAdd={id =>
                  setState(prev => ({
                    ...prev,
                    coupons: [...prev.coupons, { couponId: id }],
                  }))
                }
                onCouponRemove={id =>
                  setState(prev => ({
                    ...prev,
                    coupons: prev.coupons.filter(c => c.couponId !== id),
                  }))
                }
                availableCoupons={availableCoupons}
                isCouponAvailable={isCouponAvailableForPlan}
                currency={planQuery.data?.plan?.version?.currency}
              />
            </PageSection>

            <div className="flex gap-2 justify-end">
              <Button
                type="submit"
                variant="primary"
                disabled={!isValid}
                title={!isValid ? validationErrors.join(', ') : undefined}
              >
                Next
              </Button>
            </div>
          </>
        )}
      </form>
    </Form>
  )
}

const schema = z.object({
  planVersionId: z.string(),
  customerId: z.string(),
})
