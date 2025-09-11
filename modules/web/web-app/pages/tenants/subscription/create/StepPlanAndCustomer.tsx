import {
  Badge,
  Button,
  Card,
  CardHeader,
  CardTitle,
  Form,
  GenericFormField,
  Input,
} from '@ui/components'
import { useAtom } from 'jotai'
import { Gift, Plus, Search, Tag, X } from 'lucide-react'
import { useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { z } from 'zod'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { SubscribablePlanVersionSelect } from '@/features/plans/SubscribablePlanVersionSelect'
import { CreateSubscriptionPriceComponents } from '@/features/subscriptions/pricecomponents/CreateSubscriptionPriceComponents'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'

// TODO confirm & reset form on leave
export const StepPlanAndCustomer = () => {
  const { nextStep } = useWizard()
  const [state, setState] = useAtom(createSubscriptionAtom)
  const [searchParams] = useSearchParams()
  const [addOnSearch, setAddOnSearch] = useState('')
  const [couponSearch, setCouponSearch] = useState('')
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

  const addOnsQuery = useQuery(listAddOns, {
    pagination: {
      perPage: 100,
      page: 0,
    },
  })
  const couponsQuery = useQuery(listCoupons, {
    pagination: {
      perPage: 100,
      page: 0,
    },
    filter: ListCouponRequest_CouponFilter.ACTIVE,
  })

  const availableAddOns = addOnsQuery.data?.addOns || []
  const availableCoupons = couponsQuery.data?.coupons || []

  const filteredAddOns = availableAddOns.filter(addOn =>
    addOn.name.toLowerCase().includes(addOnSearch.toLowerCase())
  )

  const filteredCoupons = availableCoupons.filter(coupon =>
    coupon.code.toLowerCase().includes(couponSearch.toLowerCase())
  )

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
              <CreateSubscriptionPriceComponents
                planVersionId={planVersionId}
                customerId={customerId}
                onValidationChange={(valid, errors) => {
                  setIsValid(valid)
                  setValidationErrors(errors)
                }}
              />
            </PageSection>

            <PageSection
              className="fadeIn"
              header={{
                title: 'Add-ons & Discounts',
                subtitle: 'Optional enhancements and promotional offers',
              }}
            >
              {/* Add-ons Section */}
              <div className="space-y-4">
                <div>
                  <h3 className="text-sm font-medium mb-3 flex items-center gap-2">
                    <Plus className="h-4 w-4 text-green-500" />
                    Add-ons
                    <Badge variant="outline" size="sm">
                      {state.addOns.length} selected
                    </Badge>
                  </h3>

                  {state.addOns.length > 0 ? (
                    <div className="grid gap-2 mb-3">
                      {state.addOns.map(addon => {
                        const addOnData = availableAddOns.find(a => a.id === addon.addOnId)
                        return (
                          <Card key={addon.addOnId} className="border-green-200 bg-green-50/30">
                            <CardHeader className="p-3 flex flex-row items-center justify-between">
                              <div className="flex items-center gap-2">
                                <CardTitle className="text-sm">
                                  {addOnData?.name || addon.addOnId}
                                </CardTitle>
                                <Badge variant="secondary" size="sm">
                                  Add-on
                                </Badge>
                              </div>
                              <Button
                                type="button"
                                variant="ghost"
                                size="sm"
                                onClick={() => {
                                  setState(prev => ({
                                    ...prev,
                                    addOns: prev.addOns.filter(a => a.addOnId !== addon.addOnId),
                                  }))
                                }}
                              >
                                <X className="h-3 w-3" />
                              </Button>
                            </CardHeader>
                          </Card>
                        )
                      })}
                    </div>
                  ) : (
                    <p className="text-sm text-muted-foreground mb-3">No add-ons selected</p>
                  )}

                  <div className="relative">
                    <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
                    <Input
                      type="search"
                      placeholder="Search available add-ons..."
                      value={addOnSearch}
                      onChange={e => setAddOnSearch(e.target.value)}
                      className="pl-8 h-9"
                    />
                  </div>

                  {addOnSearch && filteredAddOns.length > 0 && (
                    <div className="mt-2 border rounded-md p-2 space-y-1 max-h-32 overflow-y-auto">
                      {filteredAddOns.slice(0, 5).map(addOn => {
                        const isSelected = state.addOns.some(a => a.addOnId === addOn.id)
                        return (
                          <Button
                            key={addOn.id}
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="w-full justify-start text-sm"
                            disabled={isSelected}
                            onClick={() => {
                              setState(prev => ({
                                ...prev,
                                addOns: [...prev.addOns, { addOnId: addOn.id }],
                              }))
                              setAddOnSearch('')
                            }}
                          >
                            <Plus className="h-3 w-3 mr-2" />
                            {addOn.name}
                            {isSelected && (
                              <Badge variant="secondary" size="sm" className="ml-auto">
                                Added
                              </Badge>
                            )}
                          </Button>
                        )
                      })}
                    </div>
                  )}
                </div>

                {/* Coupons Section */}
                <div className="border-t pt-4">
                  <h3 className="text-sm font-medium mb-3 flex items-center gap-2">
                    <Tag className="h-4 w-4 text-purple-500" />
                    Discount Coupons
                    <Badge variant="outline" size="sm">
                      {state.coupons.length} applied
                    </Badge>
                  </h3>

                  {state.coupons.length > 0 ? (
                    <div className="grid gap-2 mb-3">
                      {state.coupons.map(coupon => {
                        const couponData = availableCoupons.find(c => c.id === coupon.couponId)
                        return (
                          <Card key={coupon.couponId} className="bg-card text-card-foreground">
                            <CardHeader className="p-3 flex flex-row items-center justify-between">
                              <div className="flex items-center gap-2">
                                <CardTitle className="text-sm">
                                  {couponData?.code || coupon.couponId}
                                </CardTitle>
                                <Badge variant="secondary" size="sm">
                                  Discount
                                </Badge>
                              </div>
                              <Button
                                type="button"
                                variant="ghost"
                                size="sm"
                                onClick={() => {
                                  setState(prev => ({
                                    ...prev,
                                    coupons: prev.coupons.filter(
                                      c => c.couponId !== coupon.couponId
                                    ),
                                  }))
                                }}
                              >
                                <X className="h-3 w-3" />
                              </Button>
                            </CardHeader>
                          </Card>
                        )
                      })}
                    </div>
                  ) : (
                    <p className="text-sm text-muted-foreground mb-3">No coupons applied</p>
                  )}

                  <div className="relative">
                    <Search className="absolute left-2 top-2.5 h-4 w-4 text-muted-foreground" />
                    <Input
                      type="search"
                      placeholder="Search by coupon code..."
                      value={couponSearch}
                      onChange={e => setCouponSearch(e.target.value)}
                      className="pl-8 h-9"
                    />
                  </div>

                  {couponSearch && filteredCoupons.length > 0 && (
                    <div className="mt-2 border rounded-md p-2 space-y-1 max-h-32 overflow-y-auto">
                      {filteredCoupons.slice(0, 5).map(coupon => {
                        const isSelected = state.coupons.some(c => c.couponId === coupon.id)
                        return (
                          <Button
                            key={coupon.id}
                            type="button"
                            variant="ghost"
                            size="sm"
                            className="w-full justify-start text-sm"
                            disabled={isSelected}
                            onClick={() => {
                              setState(prev => ({
                                ...prev,
                                coupons: [...prev.coupons, { couponId: coupon.id }],
                              }))
                              setCouponSearch('')
                            }}
                          >
                            <Gift className="h-3 w-3 mr-2" />
                            {coupon.code}
                            {isSelected && (
                              <Badge variant="secondary" size="sm" className="ml-auto">
                                Applied
                              </Badge>
                            )}
                          </Button>
                        )
                      })}
                    </div>
                  )}
                </div>
              </div>
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
