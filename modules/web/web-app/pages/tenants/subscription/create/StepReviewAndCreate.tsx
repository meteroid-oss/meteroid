import { create } from '@bufbuild/protobuf';
import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query';
import { Skeleton } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Badge, Button, Card, CardContent, CardHeader, CardTitle } from '@ui/components'
import { useAtom } from 'jotai'
import { Calendar, Package, PlusIcon, Shield, Tag, User } from 'lucide-react'
import { useMemo } from 'react'
import { useNavigate } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { toast } from 'sonner'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { resolveEntitlementSpecs } from '@/features/entitlements/creation/resolveEntitlementSpecs'
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing'
import { InvoicePreviewCard } from '@/features/subscriptions/UpcomingInvoiceCard'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { mapDatev2 } from '@/lib/mapping'
import {
  createSubscriptionAtom,
  CreateSubscriptionState,
  PaymentMethodsConfigType,
} from '@/pages/tenants/subscription/create/state'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'
import { Coupon } from '@/rpc/api/coupons/v1/models_pb'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { createFeature } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import {
  ActivationCondition,
  BankTransferSchema,
  ExternalSchema,
  OnlinePaymentSchema,
  PaymentMethodsConfigSchema,
} from '@/rpc/api/subscriptions/v1/models_pb';
import {
  createSubscription,
  listSubscriptions,
  previewCreateSubscription,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { PaymentMethodsConfig } from '@/rpc/api/subscriptions/v1/models_pb';


// Build PaymentMethodsConfig from state (simple: just the type, no overrides)
const buildProtoPaymentMethodsConfig = (
  type: PaymentMethodsConfigType
): PaymentMethodsConfig | undefined => {
  switch (type) {
    case 'online':
      // Online without config = inherit from invoicing entity
      return create(PaymentMethodsConfigSchema, {
        config: { case: 'online', value: create(OnlinePaymentSchema) },
      })
    case 'bankTransfer':
      return create(PaymentMethodsConfigSchema, {
        config: { case: 'bankTransfer', value: create(BankTransferSchema) },
      })
    case 'external':
      return create(PaymentMethodsConfigSchema, {
        config: { case: 'external', value: create(ExternalSchema) },
      })
    default:
      return undefined
  }
}

// Builds the CreateSubscription payload shared by the create call and the server-side
// preview (Subscription Summary), so the previewed first invoice matches what creation bills.
// Entitlements are resolved separately at create time and don't affect totals, so omitted here.
const buildCreateSubscriptionMessage = (state: CreateSubscriptionState, currency: string) => {
  // Map billingDay to billingDayAnchor
  // 'FIRST' = 1st of month (fixed day), 'SUB_START_DAY' = anniversary (undefined)
  const billingDayAnchor = state.billingDay === 'FIRST' ? 1 : state.billingDayAnchor

  return {
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
  }
}

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
      : skipToken
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
      queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listSubscriptions.parent,
        cardinality: undefined
      }) })
    },
  })

  const createFeatureMutation = useMutation(createFeature)

  const currency = planQuery.data?.plan?.version?.currency

  const selectedAddOns =
    addOnsQuery.data?.addOns.filter(a => state.addOns.some(sa => sa.addOnId === a.id)) || []

  const selectedCoupons =
    couponsQuery.data?.coupons.filter(c => state.coupons.some(sc => sc.couponId === c.id)) || []

  // Mirror backend discount.rs ordering: percentage first (customer-friendly),
  // tiebreak by coupon.id. Used everywhere coupons are displayed/applied so
  // the wizard matches the order the invoice will use.
  const orderedCoupons = [...selectedCoupons].sort((a, b) => {
    const typeOf = (c: Coupon) => (c.discount?.discountType?.case === 'percentage' ? 0 : 1)
    const ta = typeOf(a)
    const tb = typeOf(b)
    if (ta !== tb) return ta - tb
    return a.id.localeCompare(b.id)
  })

  // Server-computed first-invoice preview powering the Subscription Summary. It mirrors what
  // creation will bill, so the totals reflect coupons and taxes.
  const previewInput = useMemo(
    () =>
      currency && state.customerId && state.planVersionId
        ? buildCreateSubscriptionMessage(state, currency)
        : undefined,
    [state, currency]
  )

  const previewQuery = useQuery(
    previewCreateSubscription,
    previewInput ? { subscription: previewInput } : skipToken
  )

  if (!currency) {
    return <div>Loading plan...</div>
  }

  const handleCreate = async () => {
    try {
      const resolvedEntitlements = await resolveEntitlementSpecs(
        state.entitlements,
        req => createFeatureMutation.mutateAsync(req)
      )

      const created = await createSubscriptionMutation.mutateAsync({
        subscription: {
          ...buildCreateSubscriptionMessage(state, currency),
          entitlements: resolvedEntitlements,
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
                        {orderedCoupons.map(coupon => (
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
            {env.entitlementsEnabled && state.entitlements.length > 0 && (
              <Card>
                <CardHeader className="flex flex-row items-center gap-2">
                  <Shield className="h-5 w-5" />
                  <CardTitle className="text-base">
                    Entitlements ({state.entitlements.length})
                  </CardTitle>
                </CardHeader>
                <CardContent>
                  <div className="space-y-2">
                    {state.entitlements.map((e, i) => (
                      <div key={i} className="flex items-center justify-between text-sm">
                        <span className="font-medium">{e.featureDisplayName}</span>
                        <div className="flex items-center gap-2 text-xs text-muted-foreground">
                          <span>
                            {e.featureType === 'boolean'
                              ? e.boolEnabled !== false
                                ? 'Enabled'
                                : 'Disabled'
                              : e.limit
                                ? `${e.limit} / ${e.resetPeriodType ?? 'cycle'}`
                                : 'Unlimited'}
                          </span>
                        </div>
                      </div>
                    ))}
                  </div>
                </CardContent>
              </Card>
            )}
          </div>

          {/* Right Column - Server-computed first invoice preview */}
          <div className="lg:col-span-1">
            <div className="sticky top-6 space-y-2">
              <h3 className="text-lg font-semibold">Subscription Summary</h3>
              {previewQuery.isLoading ? (
                <div className="bg-card rounded-lg border border-border shadow-sm p-4">
                  <Skeleton height={20} width={200} className="mb-2" />
                  <Skeleton height={14} width={150} />
                </div>
              ) : previewQuery.isError || !previewQuery.data?.invoice ? (
                <div className="bg-card rounded-lg border border-border shadow-sm p-4 text-sm text-muted-foreground">
                  Unable to compute the subscription summary. Review the configuration and try
                  again.
                </div>
              ) : (
                <InvoicePreviewCard
                  invoice={previewQuery.data.invoice}
                  currency={currency}
                  subscriptionId=""
                  title="First invoice"
                  defaultExpanded
                  hideUsageDetails
                />
              )}
            </div>
          </div>
        </div>
      </PageSection>

      <div className="flex gap-2 justify-end">
        <Button variant="secondary" onClick={previousStep}>
          Back
        </Button>
        <Button
          onClick={handleCreate}
          disabled={createSubscriptionMutation.isPending || createFeatureMutation.isPending}
          className="min-w-[120px]"
          variant="brand"
        >
          {createSubscriptionMutation.isPending || createFeatureMutation.isPending
            ? 'Creating...'
            : 'Create subscription'}
        </Button>
      </div>
    </div>
  )
}
