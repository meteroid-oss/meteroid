import { create } from '@bufbuild/protobuf';
import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query';
import { Alert, AlertDescription, Skeleton } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Badge, Button, Card, CardContent, CardHeader, CardTitle } from '@ui/components'
import { useAtom } from 'jotai'
import { Calendar, Package, PlusIcon, Shield, Tag, User } from 'lucide-react'
import { useMemo } from 'react'
import { useNavigate } from 'react-router'
import { useWizard } from 'react-use-wizard'
import { toast } from 'sonner'

import { PageSection } from '@/components/layouts/shared/PageSection'
import {
  MinimumCommitmentDisplay,
  productScopeOptions,
} from '@/features/billing/minimumCommitment'
import { serializeCustomProperties } from '@/features/customproperties/CustomPropertiesFields'
import { productGroups } from '@/features/entitlements/productGroups'
import { grantValueText } from '@/features/entitlements/rows/grantDisplay'
import { draftsToSpecs } from '@/features/entitlements/rows/grantDraft'
import { useTrialPlans } from '@/features/plans/trial/TrialTermsFields'
import {
  buildExistingProductRef,
  buildFeeStructure,
  buildNewProductRef,
  buildPriceInputs,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing'
import { SubscribedForSummary } from '@/features/subscriptions/SubscribedFor'
import { InvoicePreviewCard } from '@/features/subscriptions/UpcomingInvoiceCard'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { mapDatev2 } from '@/lib/mapping'
import { formatDefaultNetTerms, formatNetTerms } from '@/lib/mapping/priceToSubscriptionFee'
import { useDefaultNetTerms } from '@/pages/tenants/subscription/create/StepSettings'
import {
  createSubscriptionAtom,
  CreateSubscriptionState,
  PaymentMethodsConfigType,
} from '@/pages/tenants/subscription/create/state'
import {
  offerRequest,
  startOfferLabel,
  trialLabel,
  trialRefusal,
  trialRequest,
} from '@/pages/tenants/subscription/create/trial'
import { useSubscriptionSale } from '@/pages/tenants/subscription/create/useSubscriptionSale'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'
import { Coupon } from '@/rpc/api/coupons/v1/models_pb'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { getOffer } from '@/rpc/api/offers/v1/offers-OffersService_connectquery'
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
const buildCreateSubscriptionMessage = (
  state: CreateSubscriptionState,
  currency: string,
  option: string | undefined
) => {
  // Map billingDay to billingDayAnchor
  // 'FIRST' = 1st of month (fixed day), 'SUB_START_DAY' = anniversary (undefined)
  const billingDayAnchor = state.billingDay === 'FIRST' ? 1 : state.billingDayAnchor

  return {
    planVersionId: state.planVersionId,
    customerId: state.customerId,
    option,
    startDate: mapDatev2(state.startDate),
    endDate: state.endDate && mapDatev2(state.endDate),
    billingDayAnchor,
    netTerms: state.netTerms,
    activationCondition: state.activationCondition,
    trial: trialRequest(state),
    offer: offerRequest(state),
    invoiceMemo: state.invoiceMemo,
    invoiceThreshold: state.invoiceThreshold,
    purchaseOrder: state.purchaseOrder,
    resourceId: state.resourceId,
    autoAdvanceInvoices: state.autoAdvanceInvoices,
    chargeAutomatically: state.chargeAutomatically,
    paymentMethodsConfig: buildProtoPaymentMethodsConfig(state.paymentMethodsType),
    skipPastInvoices: state.skipPastInvoices,
    components: {
      parameterizedComponents: state.components.parameterized.map(c => ({
        componentId: c.componentId,
        initialSlotCount: c.initialSlotCount,
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
            : buildNewProductRef(c.name),
          price: priceEntries[0],
          feeStructure: buildFeeStructure(c.feeType, c.formData),
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
    minimumCommitmentOverride: state.minimumOverride,
    dropMinimumCommitment: !!state.dropMinimum,
    customProperties: serializeCustomProperties(state.customProperties),
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

  const sale = useSubscriptionSale(state)

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

  const netTermsDefault = useDefaultNetTerms(state)
  const { planName } = useTrialPlans(planQuery.data?.plan?.plan?.id)
  const catalogOfferName = useQuery(
    getOffer,
    state.startOfferMode === 'catalog' && state.startOfferId
      ? { id: state.startOfferId }
      : skipToken
  ).data?.offer?.name

  const allComponents = sale.components ?? []
  const includedComponents = allComponents.filter(c => !state.components.removed.includes(c.id))
  const currency = sale.currency
  // The minimum commitment is set in the plan's currency (StepSettings).
  const planCurrency = planQuery.data?.plan?.version?.currency

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
  // creation will bill, so the totals reflect coupons, taxes and the minimum-commitment floor.
  const previewInput = useMemo(
    () =>
      currency && state.customerId && state.planVersionId
        ? buildCreateSubscriptionMessage(state, currency, sale.handle)
        : undefined,
    [state, currency, sale.handle]
  )

  const previewQuery = useQuery(
    previewCreateSubscription,
    previewInput ? { subscription: previewInput } : skipToken
  )

  const refusal =
    trialRefusal(createSubscriptionMutation.error) ?? trialRefusal(previewQuery.error)

  if (!currency) {
    return <div>Loading plan...</div>
  }

  const handleCreate = async () => {
    try {
      const created = await createSubscriptionMutation.mutateAsync({
        subscription: {
          ...buildCreateSubscriptionMessage(state, currency, sale.handle),
          entitlements: draftsToSpecs(state.entitlements),
        },
      })
      toast.success('Subscription created successfully')
      if (created.subscription?.warnings.includes('plan_sold_for_mismatch'))
        toast.warning('Created for another target than the plan is sold for.')
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
                  {sale.options.length > 1 && sale.option && (
                    <div>
                      <div className="text-xs text-muted-foreground">Option</div>
                      <div className="text-sm font-medium">{sale.option.name}</div>
                    </div>
                  )}
                  <div>
                    <div className="text-xs text-muted-foreground">Currency</div>
                    <div className="text-sm font-medium">{currency}</div>
                  </div>
                  {state.customerId && (
                    <SubscribedForSummary
                      customerId={state.customerId}
                      customerName={customerQuery.data?.customer?.name ?? ''}
                      resourceId={state.resourceId}
                      soldForTypeId={planQuery.data?.plan?.plan?.soldForResourceTypeId}
                      metricsNotAttributed={previewQuery.data?.metricsNotAttributed}
                    />
                  )}
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
                  <div>
                    <div className="text-xs text-muted-foreground">Start offer</div>
                    <div className="font-medium">{startOfferLabel(state, catalogOfferName)}</div>
                  </div>
                  <div>
                    <div className="text-xs text-muted-foreground">Trial</div>
                    <div className="font-medium">{trialLabel(state, planName)}</div>
                    {refusal && <div className="text-xs text-destructive">{refusal}</div>}
                  </div>
                  <div>
                    <div className="text-xs text-muted-foreground">Billing Cycle</div>
                    <div className="font-medium">
                      {state.billingDay === 'FIRST' ? '1st of month' : 'Anniversary'}
                    </div>
                  </div>
                  <div>
                    <div className="text-xs text-muted-foreground">Net Terms</div>
                    <div className="font-medium">
                      {state.netTerms !== undefined
                        ? formatNetTerms(state.netTerms)
                        : netTermsDefault
                          ? formatDefaultNetTerms(netTermsDefault)
                          : '-'}
                    </div>
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

                {(state.invoiceMemo ||
                  state.invoiceThreshold ||
                  state.purchaseOrder) && (
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
                {(previewQuery.data?.unfilteredOverlaps ?? []).length > 0 && (
                  <Alert variant="warning" className="mt-4">
                    <AlertDescription className="text-xs">
                      This subscription and{' '}
                      {previewQuery
                        .data!.unfilteredOverlaps.map(overlap => overlap.planName)
                        .join(', ')}{' '}
                      are for the same {state.resourceId ? 'resource' : 'customer'} and share metrics:
                      {state.resourceId ? ' the older one bills them.' : ' both bill the same events.'}
                    </AlertDescription>
                  </Alert>
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
                  <div className="space-y-4">
                    {productGroups(state.entitlements, e => e.feature.product).map(
                      (group, _, groups) => (
                        <div key={group.key} className="space-y-2">
                          {groups.length > 1 && (
                            <div className="text-xs font-medium text-muted-foreground">
                              {group.label}
                            </div>
                          )}
                          {group.items.map(e => (
                            <div
                              key={e.spec.featureId}
                              className="flex items-center justify-between gap-3 text-sm"
                            >
                              <span className="font-medium">{e.feature.name}</span>
                              <span className="text-[13px] text-muted-foreground">
                                {grantValueText(e.spec, e.feature, 'subscription')}
                              </span>
                            </div>
                          ))}
                        </div>
                      )
                    )}
                  </div>
                </CardContent>
              </Card>
            )}
            {/* Contract terms */}
            {currency &&
              (() => {
                const planMinimum = planQuery.data?.plan?.version?.minimumCommitment
                const effectiveMinimum = state.minimumOverride ?? (state.dropMinimum ? undefined : planMinimum)
                // Only surface the card when there's something to say.
                if (!effectiveMinimum && !state.dropMinimum) return null
                const scopeOptions = productScopeOptions([
                  ...includedComponents,
                  ...state.components.extra.map(e => ({ name: e.name, productId: e.productId })),
                ])
                return (
                  <Card>
                    <CardHeader className="flex flex-row items-center gap-2">
                      <CardTitle className="text-base">Contract terms</CardTitle>
                    </CardHeader>
                    <CardContent>
                      <MinimumCommitmentDisplay
                        currency={planCurrency ?? currency}
                        components={scopeOptions}
                        commitment={effectiveMinimum}
                        badge={
                          state.minimumOverride ? (
                            <span className="inline-flex items-center rounded bg-brand/10 px-1.5 py-0.5 text-[11px] font-medium text-brand">
                              Custom
                            </span>
                          ) : !state.dropMinimum && planMinimum ? (
                            <span className="inline-flex items-center rounded border border-border px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground">
                              From plan
                            </span>
                          ) : undefined
                        }
                        emptyText="No minimum (plan default removed for this subscription)"
                      />
                    </CardContent>
                  </Card>
                )
              })()}
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
          disabled={createSubscriptionMutation.isPending}
          className="min-w-[120px]"
          variant="brand"
        >
          {createSubscriptionMutation.isPending
            ? 'Creating...'
            : 'Create subscription'}
        </Button>
      </div>
    </div>
  )
}
