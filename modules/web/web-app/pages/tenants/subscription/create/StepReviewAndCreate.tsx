import { disableQuery, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { Badge, Button, Card, CardContent, CardHeader, CardTitle } from '@ui/components'
import { useAtom } from 'jotai'
import { Calendar, Package, PlusIcon, Tag, User } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { toast } from 'sonner'

import { PageSection } from '@/components/layouts/shared/PageSection'
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
  formDataToPrice,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing'
import { getApiComponentBillingPeriodLabel } from '@/features/subscriptions/utils/billingPeriods'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { mapDatev2 } from '@/lib/mapping'
import {
  formatUsagePriceSummary,
  getComponentPricingFromPrice,
  getPrice,
  getPriceBillingLabel,
} from '@/lib/mapping/priceToSubscriptionFee'
import {
  createSubscriptionAtom,
  PaymentMethodsConfigType,
} from '@/pages/tenants/subscription/create/state'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'
import { Coupon } from '@/rpc/api/coupons/v1/models_pb'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import {
  ActivationCondition,
  BankTransfer,
  External,
  OnlinePayment,
  PaymentMethodsConfig,
} from '@/rpc/api/subscriptions/v1/models_pb'
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

  const addOnsQuery = useQuery(
    listAddOns,
    state.planVersionId
      ? {
          planVersionId: state.planVersionId,
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
    filter: ListCouponRequest_CouponFilter.ACTIVE, // TODO filter currency etc etc
  })

  const createSubscriptionMutation = useMutation(createSubscription, {
    onSuccess: async () => {
      queryClient.invalidateQueries({ queryKey: [listSubscriptions.service.typeName] })
    },
  })

  const allComponents = componentsQuery.data?.components || []
  const includedComponents = allComponents.filter(c => !state.components.removed.includes(c.id))
  const currency = planQuery.data?.plan?.version?.currency

  const selectedAddOns =
    addOnsQuery.data?.addOns.filter(a => state.addOns.some(sa => sa.addOnId === a.id)) || []

  const selectedCoupons =
    couponsQuery.data?.coupons.filter(c => state.coupons.some(sc => sc.couponId === c.id)) || []

  if (!currency) {
    return <div>Loading plan...</div>
  }

  // Build PaymentMethodsConfig from state (simple: just the type, no overrides)
  const buildProtoPaymentMethodsConfig = (
    type: PaymentMethodsConfigType
  ): PaymentMethodsConfig | undefined => {
    switch (type) {
      case 'online':
        // Online without config = inherit from invoicing entity
        return new PaymentMethodsConfig({ config: { case: 'online', value: new OnlinePayment() } })
      case 'bankTransfer':
        return new PaymentMethodsConfig({
          config: { case: 'bankTransfer', value: new BankTransfer() },
        })
      case 'external':
        return new PaymentMethodsConfig({ config: { case: 'external', value: new External() } })
      default:
        return undefined
    }
  }

  const handleCreate = async () => {
    try {
      // Map billingDay to billingDayAnchor
      // 'FIRST' = 1st of month (fixed day), 'SUB_START_DAY' = anniversary (undefined)
      const billingDayAnchor = state.billingDay === 'FIRST' ? 1 : state.billingDayAnchor

      const created = await createSubscriptionMutation.mutateAsync({
        subscription: {
          planVersionId: state.planVersionId,
          customerId: state.customerId,
          startDate: mapDatev2(state.startDate),
          endDate: state.endDate && mapDatev2(state.endDate),
          billingDayAnchor,
          netTerms: state.netTerms,
          activationCondition: state.activationCondition,
          trialDuration: state.trialDuration,
          invoiceMemo: state.invoiceMemo,
          invoiceThreshold: state.invoiceThreshold,
          purchaseOrder: state.purchaseOrder,
          autoAdvanceInvoices: state.autoAdvanceInvoices,
          chargeAutomatically: state.chargeAutomatically,
          paymentMethodsConfig: buildProtoPaymentMethodsConfig(state.paymentMethodsType),
          skipPastInvoices: state.skipPastInvoices,
          components: {
            parameterizedComponents: state.components.parameterized.map(c => ({
              componentId: c.componentId,
              initialSlotCount: c.initialSlotCount,
              billingPeriod: c.billingPeriod,
              committedCapacity: c.committedCapacity,
            })),
            overriddenComponents: state.components.overridden.map(c => {
              const pricingType = toPricingTypeFromFeeType(
                c.feeType,
                c.feeType === 'usage' ? (c.formData.usageModel as string) : undefined
              )
              const priceEntries = wrapAsNewPriceEntries(
                buildPriceInputs(pricingType, c.formData, currency)
              )
              return {
                componentId: c.componentId,
                name: c.name,
                price: priceEntries[0],
              }
            }),
            extraComponents: state.components.extra.map(c => {
              const pricingType = toPricingTypeFromFeeType(
                c.feeType,
                c.feeType === 'usage' ? (c.formData.usageModel as string) : undefined
              )
              const priceEntries = wrapAsNewPriceEntries(
                buildPriceInputs(pricingType, c.formData, currency)
              )
              return {
                name: c.name,
                product: c.productId
                  ? buildExistingProductRef(c.productId)
                  : buildNewProductRef(c.name, c.feeType, c.formData),
                price: priceEntries[0],
              }
            }),
            removeComponents: state.components.removed,
          },
          addOns: {
            addOns: state.addOns.map(a => ({
              addOnId: a.addOnId,
              quantity: a.quantity ?? 1,
              ...(a.parameterization && {
                customization: {
                  case: 'parameterization' as const,
                  value: {
                    initialSlotCount: a.parameterization.initialSlotCount,
                    billingPeriod: a.parameterization.billingPeriod,
                    committedCapacity: a.parameterization.committedCapacity,
                  },
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
      const errorMessage = error instanceof Error ? error.message : 'Failed to create subscription'
      toast.error(errorMessage)
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

    // If overridden, derive display price from formData
    if (override) {
      const displayPrice = formDataToPrice(override.feeType, override.formData, currency)
      const info = getComponentPricingFromPrice(displayPrice)
      return {
        ...info,
        isOverride: true,
        billingPeriod: undefined,
      }
    }

    // Extract base price from the component's first price
    const price = component.prices[0]
    if (!price) {
      return {
        unitPrice: 0,
        quantity: 1,
        total: 0,
        isOverride: false,
        isMetered: false,
        billingPeriod: configuration?.billingPeriod,
      }
    }

    const info = getComponentPricingFromPrice(price, {
      initialSlotCount: configuration?.initialSlotCount,
    })

    return {
      ...info,
      isOverride: false,
      billingPeriod: configuration?.billingPeriod,
    }
  }

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
        <div className="grid grid-cols-1 xl:grid-cols-3 gap-8">
          {/* Left Column - Customer & Subscription Details */}
          <div className="xl:col-span-2 space-y-6">
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
                  <div>
                    <div className="text-xs text-muted-foreground">Currency</div>
                    <div className="text-sm font-medium">{currency}</div>
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
                  <div>
                    <div className="text-xs text-muted-foreground">Auto-advance</div>
                    <div className="font-medium">{state.autoAdvanceInvoices ? 'Yes' : 'No'}</div>
                  </div>
                  <div>
                    <div className="text-xs text-muted-foreground">Charge auto.</div>
                    <div className="font-medium">{state.chargeAutomatically ? 'Yes' : 'No'}</div>
                  </div>
                  {state.skipPastInvoices && (
                    <div>
                      <div className="text-xs text-muted-foreground">Migration Mode</div>
                      <div className="font-medium">Skip past invoices</div>
                    </div>
                  )}
                </div>

                {(state.invoiceMemo || state.invoiceThreshold || state.purchaseOrder) && (
                  <div className="mt-4 pt-4 border-t space-y-2">
                    {state.purchaseOrder && (
                      <div>
                        <div className="text-xs text-muted-foreground">Purchase Order</div>
                        <div className="text-sm">{state.purchaseOrder}</div>
                      </div>
                    )}
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
                              <div className="font-medium">
                                {override?.name || component.name}
                              </div>
                              {pricing.quantity > 1 && !pricing.isMetered && (
                                <div className="text-xs text-muted-foreground">
                                  {formatPrice(pricing.unitPrice, currency)} × {pricing.quantity}
                                </div>
                              )}
                            </div>
                            <div className="text-right font-medium">
                              {pricing.isMetered ? (
                                (() => {
                                  const price = override
                                    ? formDataToPrice(override.feeType, override.formData, currency)
                                    : getPrice(component)
                                  const usage = price
                                    ? formatUsagePriceSummary(price, currency)
                                    : undefined
                                  return (
                                    <div className="flex items-center justify-end gap-1">
                                      {usage ? (
                                        <span>
                                          {usage.model && (
                                            <>
                                              <span className="text-muted-foreground">
                                                {usage.model}
                                              </span>{' '}
                                            </>
                                          )}
                                          {usage.amount}
                                        </span>
                                      ) : (
                                        <span className="text-muted-foreground">Metered</span>
                                      )}
                                      <Badge variant="secondary" size="sm">
                                        {getApiComponentBillingPeriodLabel(
                                          component,
                                          configuration
                                        )}
                                      </Badge>
                                    </div>
                                  )
                                })()
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
                        const displayPrice = formDataToPrice(
                          component.feeType,
                          component.formData,
                          currency
                        )
                        const pricing = getComponentPricingFromPrice(displayPrice)

                        return (
                          <div key={index} className="flex items-start justify-between text-sm">
                            <div className="flex-1 pr-2 min-h-9">
                              <div className="font-medium">{component.name}</div>
                              {pricing.quantity > 1 && !pricing.isMetered && (
                                <div className="text-xs text-muted-foreground">
                                  {formatPrice(pricing.unitPrice, currency)} × {pricing.quantity}
                                </div>
                              )}
                            </div>
                            <div className="text-right font-medium">
                              <div className="flex items-center justify-end gap-1">
                                {pricing.isMetered ? (
                                  (() => {
                                    const usage = formatUsagePriceSummary(displayPrice, currency)
                                    return (
                                      <span>
                                        {usage.model && (
                                          <>
                                            <span className="text-muted-foreground">
                                              {usage.model}
                                            </span>{' '}
                                          </>
                                        )}
                                        {usage.amount}
                                      </span>
                                    )
                                  })()
                                ) : (
                                  <span>
                                    {pricing.quantity > 1
                                      ? formatPrice(pricing.total, currency)
                                      : formatPrice(pricing.unitPrice, currency)}
                                  </span>
                                )}
                                <Badge variant="secondary" size="sm">
                                  {getPriceBillingLabel(displayPrice)}
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
                      {selectedAddOns.map(addOn => {
                        const price = addOn.price
                        const pricing = price ? getComponentPricingFromPrice(price) : undefined
                        return (
                          <div key={addOn.id} className="flex items-start justify-between text-sm">
                            <div className="flex-1 pr-2 min-h-9">
                              <div className="font-medium">{addOn.name}</div>
                            </div>
                            <div className="text-right font-medium">
                              {pricing && !pricing.isMetered ? (
                                <span>{formatPrice(pricing.total, currency)}</span>
                              ) : pricing?.isMetered ? (
                                <span className="text-muted-foreground">Metered</span>
                              ) : (
                                <span className="text-muted-foreground text-xs">Included</span>
                              )}
                            </div>
                          </div>
                        )
                      })}
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
                    const displayPrice = formDataToPrice(
                      component.feeType,
                      component.formData,
                      currency
                    )
                    const pricing = getComponentPricingFromPrice(displayPrice)
                    if (pricing.isMetered) {
                      hasMetered = true
                    } else {
                      subtotal += pricing.total
                    }
                  })

                  // Add add-ons
                  selectedAddOns.forEach(addOn => {
                    const price = addOn.price
                    const pricing = price ? getComponentPricingFromPrice(price) : undefined
                    if (pricing) {
                      if (pricing.isMetered) {
                        hasMetered = true
                      } else {
                        subtotal += pricing.total
                      }
                    }
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
                                className="flex justify-between text-sm text-success"
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
