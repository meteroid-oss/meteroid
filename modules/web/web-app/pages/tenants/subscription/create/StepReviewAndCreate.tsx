import { useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { Badge, Button, Card, CardContent, CardHeader, CardTitle } from '@ui/components'
import { useAtom } from 'jotai'
import { Calendar, Package, PlusIcon, Tag, User } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { toast } from 'sonner'

import { PageSection } from '@/components/layouts/shared/PageSection'
import {
  mapExtraComponentToSubscriptionComponent,
  mapOverrideComponentToSubscriptionComponent,
} from '@/features/subscriptions/pricecomponents/CreateSubscriptionPriceComponents'
import {
  getApiComponentBillingPeriodLabel,
  getExtraComponentBillingPeriodLabel,
} from '@/features/subscriptions/utils/billingPeriods'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { mapDatev2 } from '@/lib/mapping'
import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'
import { Coupon } from '@/rpc/api/coupons/v1/models_pb'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { ActivationCondition } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  createSubscription,
  listSubscriptions,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

export const StepReviewAndCreate = () => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  const { previousStep } = useWizard()
  const [state] = useAtom(createSubscriptionAtom)
  const queryClient = useQueryClient()

  // Fetch data for display
  const customerQuery = useQuery(
    getCustomerById,
    { id: state.customerId! },
    { enabled: !!state.customerId }
  )

  const planQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: state.planVersionId! },
    { enabled: !!state.planVersionId }
  )

  const componentsQuery = useQuery(
    listPriceComponents,
    { planVersionId: state.planVersionId! },
    { enabled: !!state.planVersionId }
  )

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
    filter: ListCouponRequest_CouponFilter.ACTIVE, // TODO filter currency etc etc
  })

  const createSubscriptionMutation = useMutation(createSubscription, {
    onSuccess: async () => {
      queryClient.invalidateQueries({ queryKey: [listSubscriptions.service.typeName] })
    },
  })

  const handleCreate = async () => {
    try {
      const created = await createSubscriptionMutation.mutateAsync({
        subscription: {
          planVersionId: state.planVersionId,
          customerId: state.customerId,
          startDate: mapDatev2(state.startDate),
          endDate: state.endDate && mapDatev2(state.endDate),
          billingDayAnchor: state.billingDayAnchor,
          netTerms: state.netTerms,
          activationCondition: state.activationCondition,
          trialDuration: state.trialDuration,
          invoiceMemo: state.invoiceMemo,
          invoiceThreshold: state.invoiceThreshold,
          // TODO: Add components, addOns, coupons
          components: {
            parameterizedComponents: state.components.parameterized.map(c => ({
              componentId: c.componentId,
              initialSlotCount: c.initialSlotCount,
              billingPeriod: c.billingPeriod,
              committedCapacity: c.committedCapacity,
            })),
            overriddenComponents: state.components.overridden.map(c => ({
              componentId: c.componentId,
              component: mapOverrideComponentToSubscriptionComponent(c),
            })),
            extraComponents: state.components.extra.map(c => ({
              component: mapExtraComponentToSubscriptionComponent(c),
            })),
            removeComponents: state.components.removed,
          },
          addOns: {
            addOns: state.addOns.map(a => ({
              addOnId: a.addOnId,
              ...(a.parameterization && {
                parameterization: {
                  initialSlotCount: a.parameterization.initialSlotCount,
                  billingPeriod: a.parameterization.billingPeriod,
                  committedCapacity: a.parameterization.committedCapacity,
                },
              }),
              ...(a.override && {
                override: {
                  name: a.override.name,
                  // TODO: Map fee properly
                },
              }),
            })),
          },
          coupons: {
            coupons: state.coupons.map(c => ({
              couponId: c.couponId,
            })),
          },
        },
      })
      toast.success('Subscription created successfully')
      navigate(`${basePath}/subscriptions/${created.subscription?.id}`)
    } catch (error) {
      toast.error('Failed to create subscription')
      console.error(error)
    }
  }

  const getActivationConditionText = (condition: ActivationCondition) => {
    switch (condition) {
      case ActivationCondition.ON_START:
        return 'On Start Date'
      case ActivationCondition.ON_CHECKOUT:
        return 'On Checkout'
      case ActivationCondition.MANUAL:
        return 'Manual Activation'
      default:
        return 'Unknown'
    }
  }

  const formatPrice = (price: string | number, currency: string) => {
    const amount = typeof price === 'string' ? parseFloat(price || '0') : price
    return amount.toLocaleString(undefined, {
      style: 'currency',
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })
  }

  const getComponentPricing = (component: PriceComponent) => {
    const configuration = state.components.parameterized.find(p => p.componentId === component.id)
    const override = state.components.overridden.find(o => o.componentId === component.id)

    // If overridden, return override pricing
    if (override?.fee?.data?.unitPrice) {
      const unitPrice = parseFloat(override.fee.data.unitPrice)
      const quantity = override.fee.data.quantity || 1
      return {
        unitPrice,
        quantity,
        total: unitPrice * quantity,
        isOverride: true,
        isMetered: false,
        billingPeriod: undefined,
      }
    }

    // Extract base price from component
    let unitPrice = 0
    let quantity = 1

    if (component.fee?.feeType?.value) {
      const feeType = component.fee.feeType.case
      switch (feeType) {
        case 'rate': {
          const rateFeeData = component.fee.feeType.value

          if (rateFeeData.rates && rateFeeData.rates.length > 0) {
            // Use configured billing period rate if available
            if (configuration?.billingPeriod !== undefined) {
              const rate = rateFeeData.rates.find(r => r.term === configuration.billingPeriod)
              if (rate) unitPrice = parseFloat(rate.price)
            } else {
              unitPrice = parseFloat(rateFeeData.rates[0].price)
            }
          }
          break
        }
        case 'slot': {
          const slotFeeData = component.fee.feeType.value
          if (slotFeeData.rates && slotFeeData.rates.length > 0) {
            // Use configured billing period rate if available
            if (configuration?.billingPeriod !== undefined) {
              const rate = slotFeeData.rates.find(r => r.term == configuration.billingPeriod)

              if (rate) unitPrice = parseFloat(rate.price)
            } else {
              unitPrice = parseFloat(slotFeeData.rates[0].price)
            }
          }
          quantity = configuration?.initialSlotCount || 1
          break
        }

        case 'capacity': {
          const capacityFeeData = component.fee.feeType.value
          if (capacityFeeData.thresholds && capacityFeeData.thresholds.length > 0) {
            if (configuration?.committedCapacity !== undefined) {
              const threshold = capacityFeeData.thresholds.find(
                t => BigInt(t.includedAmount) === configuration.committedCapacity
              )
              if (threshold) {
                unitPrice = parseFloat(threshold.price)
              }
            } else {
              unitPrice = parseFloat(capacityFeeData.thresholds[0].price)
            }
          }
          break
        }
        case 'usage':
          // Usage pricing is complex (tiers, blocks, per unit, package, matrix)
          // we don't try to calculate a simple price - will be marked as metered
          return {
            unitPrice: 0,
            quantity: 1,
            total: 0,
            isOverride: false,
            isMetered: true,
            billingPeriod: undefined,
          }
        case 'oneTime': {
          const oneTimeFeeData = component.fee.feeType.value
          unitPrice = parseFloat(oneTimeFeeData.unitPrice)
          break
        }

        case 'extraRecurring': {
          const recFeeData = component.fee.feeType.value
          unitPrice = parseFloat(recFeeData.unitPrice)
          break
        }
      }
    }

    return {
      unitPrice,
      quantity,
      total: unitPrice * quantity,
      isOverride: false,
      isMetered: false,
      billingPeriod: configuration?.billingPeriod,
    }
  }

  const allComponents = componentsQuery.data?.components || []
  const includedComponents = allComponents.filter(c => !state.components.removed.includes(c.id))
  const currency = customerQuery.data?.customer?.currency || 'USD'

  const selectedAddOns =
    addOnsQuery.data?.addOns.filter(a => state.addOns.some(sa => sa.addOnId === a.id)) || []

  const selectedCoupons =
    couponsQuery.data?.coupons.filter(c => state.coupons.some(sc => sc.couponId === c.id)) || []

  console.log('includedComponents', includedComponents)
  console.log('selectedCoupons', selectedCoupons)

  // Calculate coupon discount
  const getCouponDiscount = (coupon: Coupon, subtotal: number) => {
    if (!coupon.discount) return 0

    // Handle different discount types
    if (coupon.discount.discountType?.case === 'percentage') {
      const percentage = parseFloat(coupon.discount.discountType.value?.percentage || '0')
      return (subtotal * percentage) / 100
    } else if (coupon.discount.discountType?.case === 'fixed') {
      return parseFloat(coupon.discount.discountType.value?.amount || '0')
    }

    return 0
  }
  return (
    <div className="space-y-6">
      <PageSection
        header={{
          title: 'Review & Create Subscription',
          subtitle: 'Review all configuration before creating the subscription',
        }}
      >
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
          {/* Left Column - Customer & Subscription Details */}
          <div className="lg:col-span-2 space-y-6">
            {/* Customer & Plan Info */}
            <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
              <Card>
                <CardHeader className="flex flex-row items-center gap-2">
                  <User className="h-5 w-5" />
                  <CardTitle className="text-base">Customer</CardTitle>
                </CardHeader>
                <CardContent className="space-y-3">
                  <div>
                    <div className="text-sm font-medium">
                      {customerQuery.data?.customer?.name || 'Loading...'}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {customerQuery.data?.customer?.id}
                    </div>
                  </div>
                  <div>
                    <div className="text-xs text-muted-foreground">Currency</div>
                    <div className="text-sm font-medium">{currency}</div>
                  </div>
                </CardContent>
              </Card>

              <Card>
                <CardHeader className="flex flex-row items-center gap-2">
                  <Package className="h-5 w-5" />
                  <CardTitle className="text-base">Plan</CardTitle>
                </CardHeader>
                <CardContent className="space-y-3">
                  <div>
                    <div className="text-sm font-medium">
                      {planQuery.data?.plan?.plan?.name || 'Loading...'}
                    </div>
                    <div className="text-xs text-muted-foreground">
                      {planQuery.data?.plan?.plan?.description}
                    </div>
                  </div>
                </CardContent>
              </Card>
            </div>

            {/* Timeline & Settings */}
            <Card>
              <CardHeader className="flex flex-row items-center gap-2">
                <Calendar className="h-5 w-5" />
                <CardTitle className="text-base">Subscription Details</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="grid grid-cols-2 md:grid-cols-3 gap-4 text-sm">
                  <div>
                    <div className="text-xs text-muted-foreground">Start Date</div>
                    <div className="font-medium">{state.startDate.toLocaleDateString()}</div>
                  </div>
                  {state.endDate && (
                    <div>
                      <div className="text-xs text-muted-foreground">End Date</div>
                      <div className="font-medium">{state.endDate.toLocaleDateString()}</div>
                    </div>
                  )}
                  {state.trialDuration && (
                    <div>
                      <div className="text-xs text-muted-foreground">Trial Period</div>
                      <div className="font-medium">{state.trialDuration} days</div>
                    </div>
                  )}
                  <div>
                    <div className="text-xs text-muted-foreground">Billing Cycle</div>
                    <div className="font-medium">
                      {state.billingDay === 'FIRST' ? '1st of month' : 'Anniversary'}
                    </div>
                  </div>
                  <div>
                    <div className="text-xs text-muted-foreground">Net Terms</div>
                    <div className="font-medium">{state.netTerms} days</div>
                  </div>
                  <div>
                    <div className="text-xs text-muted-foreground">Activation</div>
                    <div className="font-medium">
                      {getActivationConditionText(state.activationCondition)}
                    </div>
                  </div>
                </div>

                {(state.invoiceMemo || state.invoiceThreshold) && (
                  <div className="mt-4 pt-4 border-t space-y-2">
                    {state.invoiceMemo && (
                      <div>
                        <div className="text-xs text-muted-foreground">Invoice Memo</div>
                        <div className="text-sm">{state.invoiceMemo}</div>
                      </div>
                    )}
                    {state.invoiceThreshold && (
                      <div>
                        <div className="text-xs text-muted-foreground">Invoice Threshold</div>
                        <div className="text-sm">{state.invoiceThreshold}</div>
                      </div>
                    )}
                  </div>
                )}
              </CardContent>
            </Card>

            {/* Add-ons & Coupons */}
            {(selectedAddOns.length > 0 || selectedCoupons.length > 0) && (
              <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                {selectedAddOns.length > 0 && (
                  <Card>
                    <CardHeader className="flex flex-row items-center gap-2">
                      <PlusIcon className="h-5 w-5" />
                      <CardTitle className="text-base">Add-ons ({selectedAddOns.length})</CardTitle>
                    </CardHeader>
                    <CardContent>
                      <div className="space-y-2">
                        {selectedAddOns.map(addOn => {
                          // const config = state.addOns.find(a => a.addOnId === addOn.id)
                          return (
                            <div key={addOn.id} className="flex items-center justify-between">
                              <span className="text-sm">{addOn.name}</span>
                            </div>
                          )
                        })}
                      </div>
                    </CardContent>
                  </Card>
                )}

                {selectedCoupons.length > 0 && (
                  <Card>
                    <CardHeader className="flex flex-row items-center gap-2">
                      <Tag className="h-5 w-5" />
                      <CardTitle className="text-base">
                        Coupons ({selectedCoupons.length})
                      </CardTitle>
                    </CardHeader>
                    <CardContent>
                      <div className="space-y-2">
                        {selectedCoupons.map(coupon => (
                          <div key={coupon.id} className="flex items-center justify-between">
                            <span className="text-sm">{coupon.code}</span>
                            <Badge variant="secondary" size="sm">
                              Applied
                            </Badge>
                          </div>
                        ))}
                      </div>
                    </CardContent>
                  </Card>
                )}
              </div>
            )}
          </div>

          {/* Right Column - Invoice-style Pricing */}
          <div className="lg:col-span-1">
            <Card className="sticky top-6">
              <CardHeader>
                <CardTitle className="text-lg">Subscription Summary</CardTitle>
              </CardHeader>
              <CardContent className="space-y-6">
                {/* Plan Components */}
                {includedComponents.length > 0 && (
                  <div>
                    <h4 className="font-medium text-xs mb-3">Plan Components</h4>
                    <div className="space-y-2">
                      {includedComponents.map(component => {
                        const pricing = getComponentPricing(component)
                        const configuration = state.components.parameterized.find(
                          p => p.componentId === component.id
                        )
                        const override = state.components.overridden.find(
                          o => o.componentId === component.id
                        )

                        return (
                          <div
                            key={component.id}
                            className="flex items-start justify-between text-sm"
                          >
                            <div className="flex-1 pr-2 min-h-9">
                              <div className="font-medium">{override?.name || component.name}</div>
                              {pricing.quantity > 1 && !pricing.isMetered && (
                                <div className="text-xs text-muted-foreground">
                                  {formatPrice(pricing.unitPrice, currency)} × {pricing.quantity}
                                </div>
                              )}
                            </div>
                            <div className="text-right font-medium">
                              {pricing.isMetered ? (
                                <div className="flex items-center justify-end gap-1">
                                  <span className="text-muted-foreground text-xs">Metered</span>
                                  <Badge variant="secondary" size="sm">
                                    Monthly
                                  </Badge>
                                </div>
                              ) : (
                                <div className="flex items-center justify-end gap-1">
                                  <span>
                                    {pricing.quantity > 1
                                      ? formatPrice(pricing.total, currency)
                                      : formatPrice(pricing.unitPrice, currency)}
                                  </span>
                                  <Badge variant="secondary" size="sm">
                                    {getApiComponentBillingPeriodLabel(component, configuration)}
                                  </Badge>
                                </div>
                              )}
                            </div>
                          </div>
                        )
                      })}
                    </div>
                  </div>
                )}

                {/* Extra Components */}
                {state.components.extra.length > 0 && (
                  <div>
                    <h4 className="font-medium text-xs mb-3">Extra Components</h4>
                    <div className="space-y-2">
                      {state.components.extra.map((component, index) => {
                        const unitPrice = parseFloat(component.fee?.data?.unitPrice || '0')
                        const quantity = component.fee?.data?.quantity || 1
                        const total = unitPrice * quantity

                        return (
                          <div key={index} className="flex items-start justify-between text-sm">
                            <div className="flex-1 pr-2 min-h-9">
                              <div className="font-medium">{component.name}</div>
                              {quantity > 1 && (
                                <div className="text-xs text-muted-foreground">
                                  {formatPrice(unitPrice, currency)} × {quantity}
                                </div>
                              )}
                            </div>
                            <div className="text-right font-medium">
                              <div className="flex items-center justify-end gap-1">
                                <span>
                                  {quantity > 1
                                    ? formatPrice(total, currency)
                                    : formatPrice(unitPrice, currency)}
                                </span>
                                <Badge variant="secondary" size="sm">
                                  {getExtraComponentBillingPeriodLabel(component.fee?.fee)}
                                </Badge>
                              </div>
                            </div>
                          </div>
                        )
                      })}
                    </div>
                  </div>
                )}

                {/* Add-ons */}
                {selectedAddOns.length > 0 && (
                  <div>
                    <h4 className="font-medium text-xs mb-3">Add-ons</h4>
                    <div className="space-y-2">
                      {selectedAddOns.map(addOn => (
                        <div key={addOn.id} className="flex items-start justify-between text-sm">
                          <div className="flex-1 pr-2 min-h-9">
                            <div className="font-medium">{addOn.name}</div>
                          </div>
                          <div className="text-right font-medium text-muted-foreground text-xs">
                            Included
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {/* Calculate totals */}
                {(() => {
                  let subtotal = 0
                  let hasMetered = false

                  // Add plan components
                  includedComponents.forEach(component => {
                    const pricing = getComponentPricing(component)
                    if (pricing.isMetered) {
                      hasMetered = true
                    } else {
                      subtotal += pricing.total
                    }
                  })

                  // Add extra components
                  state.components.extra.forEach(component => {
                    const unitPrice = parseFloat(component.fee?.data?.unitPrice || '0')
                    const quantity = component.fee?.data?.quantity || 1
                    subtotal += unitPrice * quantity
                  })

                  // Calculate total discount from coupons
                  let totalDiscount = 0
                  selectedCoupons.forEach(coupon => {
                    totalDiscount += getCouponDiscount(coupon, subtotal)
                  })

                  const finalTotal = Math.max(0, subtotal - totalDiscount)

                  return (
                    <div className="border-t pt-4 space-y-2">
                      <div className="flex justify-between text-sm">
                        <span>Subtotal (Fixed)</span>
                        <span className="font-medium">{formatPrice(subtotal, currency)}</span>
                      </div>

                      {/* Show discounts after subtotal */}
                      {selectedCoupons.length > 0 && totalDiscount > 0 && (
                        <div className="space-y-1">
                          {selectedCoupons.map(coupon => {
                            const discount = getCouponDiscount(coupon, subtotal)
                            if (discount <= 0) return null
                            return (
                              <div
                                key={coupon.id}
                                className="flex justify-between text-sm text-green-600"
                              >
                                <span>- {coupon.code}</span>
                                <span>-{formatPrice(discount, currency)}</span>
                              </div>
                            )
                          })}
                        </div>
                      )}

                      {hasMetered && (
                        <div className="flex justify-between text-sm text-muted-foreground">
                          <span>+ Usage</span>
                          <span>Metered</span>
                        </div>
                      )}

                      <div className="flex justify-between text-base font-semibold border-t pt-2">
                        <span>First Invoice (excl. tax)</span>
                        <span>
                          {finalTotal > 0
                            ? hasMetered
                              ? `${formatPrice(finalTotal, currency)} + usage`
                              : formatPrice(finalTotal, currency)
                            : hasMetered
                              ? 'Usage only'
                              : formatPrice(0, currency)}
                        </span>
                      </div>
                    </div>
                  )
                })()}

                {/* Excluded Components */}
                {state.components.removed.length > 0 && (
                  <div className="border-t pt-4">
                    <h4 className="font-medium text-sm mb-2 text-muted-foreground">
                      Excluded Components
                    </h4>
                    <div className="space-y-1">
                      {state.components.removed.map(componentId => {
                        const component = allComponents.find(c => c.id === componentId)
                        return (
                          <div
                            key={componentId}
                            className="flex items-center justify-between text-xs text-muted-foreground"
                          >
                            <span className="line-through">{component?.name || componentId}</span>
                            <Badge variant="destructive" size="sm">
                              Excluded
                            </Badge>
                          </div>
                        )
                      })}
                    </div>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        </div>
      </PageSection>

      <div className="flex gap-2 justify-end">
        <Button variant="secondary" onClick={previousStep}>
          Back
        </Button>
        <Button
          onClick={handleCreate}
          disabled={createSubscriptionMutation.isPending}
          className="min-w-[120px]"
          variant="brand"
        >
          {createSubscriptionMutation.isPending ? 'Creating...' : 'Create subscription'}
        </Button>
      </div>
    </div>
  )
}
