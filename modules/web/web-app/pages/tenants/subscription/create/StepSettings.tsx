import { skipToken } from '@connectrpc/connect-query'
import {
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  DatePicker,
  Form,
  GenericFormField,
  InputFormField,
  Label,
  RadioGroup,
  RadioGroupItem,
  SelectFormField,
  SelectItem,
  Switch,
  TextareaFormField,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@ui/components'
import { useAtom } from 'jotai'
import { InfoIcon } from 'lucide-react'
import { useEffect } from 'react'
import { useWizard } from 'react-use-wizard'
import { z } from 'zod'

import { ChoiceCardsFormField } from '@/components/form'
import { PageSection } from '@/components/layouts/shared/PageSection'
import { Slot, hasSlot } from '@/edition'
import { scopeOptionsWithPending } from '@/features/billing/minimumCommitment'
import { CustomPropertiesFields } from '@/features/customproperties/CustomPropertiesFields'
import { useGrantNames } from '@/features/offers/GrantChips'
import { grantsFromProto } from '@/features/offers/grants'
import { offerSummary } from '@/features/offers/offers'
import { useTrialPlans } from '@/features/plans/trial/TrialTermsFields'
import { THIS_PLAN, toTrialForm, trialChips } from '@/features/plans/trial/trialTerms'
import { useResourceTypes } from '@/features/resources/hooks'
import { SubscribedFor } from '@/features/subscriptions/SubscribedFor'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { defaultNetTerms, formatDefaultNetTerms } from '@/lib/mapping/priceToSubscriptionFee'
import {
  CreateSubscriptionState,
  createSubscriptionAtom,
  effectiveSubscriptionPrices,
} from '@/pages/tenants/subscription/create/state'
import { useSubscriptionSale } from '@/pages/tenants/subscription/create/useSubscriptionSale'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { getInvoicingEntityProviders } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import {
  getOffer,
  getPlanStartOffer,
} from '@/rpc/api/offers/v1/offers-OffersService_connectquery'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { ActivationCondition } from '@/rpc/api/subscriptions/v1/models_pb'

import type { ChoiceCardOption } from '@/components/form'
import type { StartOfferMode, TrialMode } from '@/pages/tenants/subscription/create/trial'

const activationConditionToString = (
  condition: ActivationCondition
): 'ON_START' | 'ON_CHECKOUT' | 'MANUAL' => {
  switch (condition) {
    case ActivationCondition.ON_START:
      return 'ON_START'
    case ActivationCondition.ON_CHECKOUT:
      return 'ON_CHECKOUT'
    case ActivationCondition.MANUAL:
      return 'MANUAL'
    default:
      return 'ON_START'
  }
}

const activationConditionFromString = (
  condition: 'ON_START' | 'ON_CHECKOUT' | 'MANUAL'
): ActivationCondition => {
  switch (condition) {
    case 'ON_START':
      return ActivationCondition.ON_START
    case 'ON_CHECKOUT':
      return ActivationCondition.ON_CHECKOUT
    case 'MANUAL':
      return ActivationCondition.MANUAL
    default:
      return ActivationCondition.ON_START
  }
}

/** The prices the subscription will bill, from the plan, the wizard's component changes and add-ons. */
const useEffectivePrices = (state: CreateSubscriptionState) => {
  const planQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: state.planVersionId! },
    { enabled: !!state.planVersionId }
  )
  const sale = useSubscriptionSale(state)
  const addOnsQuery = useQuery(
    listAddOns,
    state.planVersionId
      ? { planVersionId: state.planVersionId, pagination: { perPage: 100, page: 0 } }
      : skipToken
  )
  const version = planQuery.data?.plan?.version
  if (!version || !sale.components || !sale.currency || !addOnsQuery.data) return undefined
  const addOnPrices = state.addOns.map(
    selected => addOnsQuery.data.addOns.find(a => a.id === selected.addOnId)?.price
  )
  return {
    prices: effectiveSubscriptionPrices(
      sale.components,
      state.components,
      addOnPrices,
      sale.currency
    ),
    planNetTerms: version.netTerms,
  }
}

/** The net terms the server applies when none are typed in; undefined while loading. */
export const useDefaultNetTerms = (state: CreateSubscriptionState) => {
  const effective = useEffectivePrices(state)
  return effective && defaultNetTerms(effective.prices, effective.planNetTerms)
}

export const StepSettings = () => {
  const { previousStep, nextStep } = useWizard()
  const [state, setState] = useAtom(createSubscriptionAtom)
  const { hasTypes } = useResourceTypes()

  // Fetch customer to get invoicing entity ID
  const customerQuery = useQuery(
    getCustomerById,
    { id: state.customerId! },
    { enabled: !!state.customerId }
  )

  // Fetch plan version to get trial config
  const planQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: state.planVersionId! },
    { enabled: !!state.planVersionId }
  )

  const planTrial = planQuery.data?.plan?.trial?.terms
  const planType = planQuery.data?.plan?.plan?.planType
  const isFreePlan = planType === PlanType.FREE
  const planCurrency = planQuery.data?.plan?.version?.currency
  const planId = planQuery.data?.plan?.plan?.id

  // Plan components for the minimum-commitment scope picker (same query key as elsewhere → cache hit).
  const minimumComponentsQuery = useQuery(
    listPriceComponents,
    { planVersionId: state.planVersionId! },
    { enabled: !!state.planVersionId && hasSlot('subscriptionCreate.minimumCommitment') }
  )

  const effectivePrices = useEffectivePrices(state)
  const netTermsDefault =
    effectivePrices && defaultNetTerms(effectivePrices.prices, effectivePrices.planNetTerms)

  const { planName, featurePlans } = useTrialPlans(planId)

  const invoicingEntityId = customerQuery.data?.customer?.invoicingEntityId

  // Fetch invoicing entity providers to check if online payment is available
  const providersQuery = useQuery(
    getInvoicingEntityProviders,
    invoicingEntityId ? { id: invoicingEntityId } : skipToken
  )

  // Check if online payment providers (card or direct debit) are configured
  const hasOnlinePaymentProvider =
    !!providersQuery.data?.cardProvider || !!providersQuery.data?.directDebitProvider

  // Loading state - default to true until we know for sure
  const isLoadingProviders = customerQuery.isLoading || providersQuery.isLoading

  const methods = useZodForm({
    schema: schema,
    defaultValues: {
      fromDate: state.startDate,
      toDate: state.endDate,
      billingDay: state.billingDay,
      trialMode: state.trialMode,
      trialDuration: state.trialDuration,
      trialUntil: state.trialUntil,
      trialFeaturesOf: state.trialFeaturesOf ?? THIS_PLAN,
      startOfferMode: state.startOfferMode,
      startOfferId: state.startOfferId,
      activationCondition:
        state.activationCondition !== undefined
          ? activationConditionToString(state.activationCondition)
          : 'ON_START',
      paymentMethodsType: state.paymentMethodsType,
      netTerms: state.netTerms,
      invoiceMemo: state.invoiceMemo,
      invoiceThreshold: state.invoiceThreshold,
      purchaseOrder: state.purchaseOrder,
      autoAdvanceInvoices: state.autoAdvanceInvoices,
      chargeAutomatically: state.chargeAutomatically,
      skipPastInvoices: state.skipPastInvoices,
      customProperties: state.customProperties,
    },
  })

  // Watch activation condition, payment methods type, charge automatically, and fromDate for cross-validation
  const [trialMode, trialUntil, startOfferMode, startOfferId] = methods.watch([
    'trialMode',
    'trialUntil',
    'startOfferMode',
    'startOfferId',
  ])
  const [activationCondition, paymentMethodsType, chargeAutomatically, fromDate] = methods.watch([
    'activationCondition',
    'paymentMethodsType',
    'chargeAutomatically',
    'fromDate',
  ])

  const { getValues, setValue, getFieldState } = methods

  const startOfferQuery = useQuery(getPlanStartOffer, planId ? { planId } : skipToken)
  const planStartOffer = startOfferQuery.data?.startOffer?.offer
  const catalogOffer = useQuery(
    getOffer,
    startOfferMode === 'catalog' && startOfferId ? { id: startOfferId } : skipToken
  ).data?.offer
  const offerGrants = grantsFromProto(planStartOffer?.grants ?? [])
  const grantNames = useGrantNames(offerGrants)
  // The trial "The plan's" or "The offer's" gives: the chosen start offer's.
  const offerTrial = startOfferMode === 'catalog' ? catalogOffer?.trial : planTrial

  // A plan without a start offer has none to give, nor a catalog offer without a trial.
  const startOfferLoaded = startOfferQuery.isSuccess
  useEffect(() => {
    if (startOfferLoaded && !planStartOffer && getValues('startOfferMode') === 'plan') {
      setValue('startOfferMode', 'none')
    }
  }, [startOfferLoaded, planStartOffer, getValues, setValue])
  useEffect(() => {
    if (catalogOffer && !catalogOffer.trial && getValues('trialMode') === 'plan') {
      setValue('trialMode', 'none')
    }
  }, [catalogOffer, getValues, setValue])

  // A plan without a trial has none to give; a trial of its own starts from the plan's features.
  const planLoaded = planQuery.isSuccess
  const planTrialFeatures = planTrial?.trialPlanId
  useEffect(() => {
    if (!planLoaded) return
    const offersPlanTrial = getValues('startOfferMode') !== 'catalog'
    if (!planTrial && offersPlanTrial && getValues('trialMode') === 'plan') {
      setValue('trialMode', 'none')
    }
    if (
      planTrialFeatures &&
      state.trialFeaturesOf === undefined &&
      !getFieldState('trialFeaturesOf').isDirty
    ) {
      setValue('trialFeaturesOf', planTrialFeatures)
    }
  }, [
    planLoaded,
    planTrial,
    planTrialFeatures,
    state.trialFeaturesOf,
    getValues,
    setValue,
    getFieldState,
  ])

  const startOfferOptions: ChoiceCardOption<StartOfferMode>[] = [
    {
      value: 'plan',
      label: "The plan's",
      hint: planStartOffer ? offerSummary(planStartOffer, grantNames) : 'None on this plan',
      disabled: startOfferLoaded && !planStartOffer,
    },
    { value: 'none', label: 'None' },
    ...(hasSlot('subscriptionCreate.offer')
      ? [{ value: 'catalog' as const, label: 'Catalog offer', hint: 'In place of the plan’s' }]
      : []),
  ]

  const awaitingOffer = startOfferMode === 'catalog' && !catalogOffer
  const trialOptions: ChoiceCardOption<TrialMode>[] = [
    {
      value: 'plan',
      label: startOfferMode === 'catalog' ? "The offer's" : "The plan's",
      hint: offerTrial
        ? trialChips(toTrialForm(offerTrial), { isFreePlan, planName })
            .slice(0, 3)
            .map(chip => chip.label)
            .join(' · ')
        : awaitingOffer
          ? 'Pick an offer'
          : 'None',
      disabled: !offerTrial && !awaitingOffer,
    },
    { value: 'none', label: 'None' },
    { value: 'custom', label: 'Custom', hint: 'Days or end date' },
  ]

  // Auto-disable chargeAutomatically and reset activationCondition when no online provider
  useEffect(() => {
    if (!isLoadingProviders && !hasOnlinePaymentProvider) {
      const currentActivation = getValues('activationCondition')
      const currentCharge = getValues('chargeAutomatically')

      if (currentActivation === 'ON_CHECKOUT') {
        setValue('activationCondition', 'ON_START')
      }
      if (currentCharge) {
        setValue('chargeAutomatically', false)
      }
    }
  }, [hasOnlinePaymentProvider, isLoadingProviders, getValues, setValue])

  // Auto-set payment methods type to Online when OnCheckout is selected
  useEffect(() => {
    if (activationCondition === 'ON_CHECKOUT' && paymentMethodsType !== 'online') {
      setValue('paymentMethodsType', 'online')
    }
  }, [activationCondition, paymentMethodsType, setValue])

  // Auto-disable chargeAutomatically when Bank or External payment methods selected
  useEffect(() => {
    if (
      (paymentMethodsType === 'bankTransfer' || paymentMethodsType === 'external') &&
      chargeAutomatically
    ) {
      setValue('chargeAutomatically', false)
    }
  }, [paymentMethodsType, chargeAutomatically, setValue])

  // Auto-disable skipPastInvoices when activation condition is not ON_START
  useEffect(() => {
    if (activationCondition !== 'ON_START' && getValues('skipPastInvoices')) {
      setValue('skipPastInvoices', false)
    }
  }, [activationCondition, getValues, setValue])

  // Reset skipPastInvoices when fromDate changes to future or is undefined
  useEffect(() => {
    if (!fromDate || fromDate >= new Date()) {
      setValue('skipPastInvoices', false)
    }
  }, [fromDate, setValue])

  const onSubmit = async (data: z.infer<typeof schema>) => {
    setState({
      ...state,
      startDate: data.fromDate,
      endDate: data.toDate,
      billingDay: data.billingDay,
      trialMode: data.trialMode,
      trialDuration: data.trialMode === 'custom' && !data.trialUntil ? data.trialDuration : undefined,
      trialUntil: data.trialMode === 'custom' ? data.trialUntil : undefined,
      trialFeaturesOf: data.trialMode === 'custom' ? data.trialFeaturesOf : undefined,
      startOfferMode: data.startOfferMode,
      startOfferId: data.startOfferMode === 'catalog' ? data.startOfferId : undefined,
      activationCondition: activationConditionFromString(data.activationCondition),
      paymentMethodsType: data.paymentMethodsType,
      netTerms: data.netTerms,
      invoiceMemo: data.invoiceMemo,
      invoiceThreshold: data.invoiceThreshold,
      purchaseOrder: data.purchaseOrder,
      autoAdvanceInvoices: data.autoAdvanceInvoices,
      chargeAutomatically: data.chargeAutomatically,
      skipPastInvoices: data.skipPastInvoices,
      customProperties: data.customProperties,
    })
    nextStep()
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-6">
        {hasTypes && state.customerId && (
          <PageSection className="fadeIn" header={{ title: 'Subscribed for' }}>
            <SubscribedFor
              customerId={state.customerId}
              customerName={customerQuery.data?.customer?.name ?? 'The customer'}
              planName={planQuery.data?.plan?.plan?.name}
              soldForTypeId={planQuery.data?.plan?.plan?.soldForResourceTypeId}
              value={state.resourceId}
              onChange={resourceId => setState(prev => ({ ...prev, resourceId }))}
            />
          </PageSection>
        )}
        {/* Subscription Timing */}
        <PageSection
          className="fadeIn"
          header={{
            title: 'Subscription Timeline',
            subtitle: 'Configure when the subscription starts and its lifecycle',
          }}
        >
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Subscription Dates</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="Start date"
                  name="fromDate"
                  render={({ field }) => (
                    <DatePicker
                      mode="single"
                      captionLayout="dropdown"
                      className="min-w-[12em]"
                      date={field.value}
                      onSelect={field.onChange}
                    />
                  )}
                />
                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="End date (optional)"
                  name="toDate"
                  render={({ field }) => (
                    <DatePicker
                      mode="single"
                      captionLayout="dropdown"
                      className="min-w-[12em]"
                      placeholder="No end date"
                      date={field.value}
                      onSelect={field.onChange}
                    />
                  )}
                />
                {/* Show only when start date is in the past */}
                {fromDate && fromDate < new Date(new Date().setHours(0, 0, 0, 0)) && (
                  <div className="pt-4 border-t space-y-2">
                    <div className="flex items-center gap-1">
                      <Label
                        className={`text-sm font-medium ${activationCondition !== 'ON_START' ? 'text-muted-foreground' : ''}`}
                      >
                        Migration Settings
                      </Label>
                      <TooltipProvider>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                          </TooltipTrigger>
                          <TooltipContent multiline className="max-w-xs">
                            <p>
                              {activationCondition !== 'ON_START'
                                ? 'Requires "On Start Date" activation condition.'
                                : 'For migrations from another billing system.'}
                            </p>
                          </TooltipContent>
                        </Tooltip>
                      </TooltipProvider>
                    </div>
                    <GenericFormField
                      control={methods.control}
                      name="skipPastInvoices"
                      render={({ field }) => (
                        <div className="space-y-1">
                          <div className="flex items-center space-x-2">
                            <Switch
                              id="skipPastInvoices"
                              checked={field.value}
                              onCheckedChange={field.onChange}
                              disabled={activationCondition !== 'ON_START'}
                            />
                            <Label
                              htmlFor="skipPastInvoices"
                              className={`font-normal text-sm ${activationCondition !== 'ON_START' ? 'text-muted-foreground' : ''}`}
                            >
                              Skip past invoices
                              {activationCondition !== 'ON_START' && ' (requires On Start Date)'}
                            </Label>
                          </div>
                          <p className="text-xs text-muted-foreground">
                            {field.value
                              ? 'Only future invoices will be emitted (including any arrear fees for the current period).'
                              : 'Off: All periods since the start date will be invoiced, including past ones.'}
                          </p>
                        </div>
                      )}
                    />
                  </div>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-base">Start offer</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <ChoiceCardsFormField
                  control={methods.control}
                  name="startOfferMode"
                  columns={3}
                  options={startOfferOptions}
                />
                {startOfferMode === 'catalog' && planId && (
                  <div className="space-y-1">
                    <Slot
                      name="subscriptionCreate.offer"
                      planId={planId}
                      value={startOfferId}
                      onChange={id =>
                        setValue('startOfferId', id, { shouldDirty: true, shouldValidate: true })
                      }
                    />
                    {methods.formState.errors.startOfferId && (
                      <p className="text-xs text-destructive">
                        {methods.formState.errors.startOfferId.message}
                      </p>
                    )}
                  </div>
                )}
                <ChoiceCardsFormField
                  control={methods.control}
                  name="trialMode"
                  label="Trial"
                  columns={3}
                  options={trialOptions}
                />
                {trialMode === 'custom' && (
                  <div className="grid grid-cols-2 gap-3">
                    <InputFormField
                      name="trialDuration"
                      label="Length"
                      type="number"
                      rightText="days"
                      placeholder={String(planTrial?.durationDays ?? 14)}
                      control={methods.control}
                      disabled={!!trialUntil}
                    />
                    <GenericFormField
                      control={methods.control}
                      layout="vertical"
                      label="Or ends on"
                      name="trialUntil"
                      render={({ field }) => (
                        <DatePicker
                          mode="single"
                          captionLayout="dropdown"
                          className="w-full"
                          placeholder="Pick a date"
                          clearable
                          date={field.value}
                          onSelect={field.onChange}
                        />
                      )}
                    />
                    <SelectFormField
                      name="trialFeaturesOf"
                      label="Features of"
                      control={methods.control}
                      containerClassName="col-span-2"
                    >
                      <SelectItem value={THIS_PLAN}>This plan</SelectItem>
                      {featurePlans.map(plan => (
                        <SelectItem key={plan.value} value={plan.value}>
                          {plan.label}
                        </SelectItem>
                      ))}
                    </SelectFormField>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        </PageSection>

        {/* Billing Configuration */}
        <PageSection
          header={{
            title: 'Billing Configuration',
            subtitle: 'Set billing cycle and payment terms',
          }}
        >
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Billing Cycle</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="Billing cycle"
                  name="billingDay"
                  render={({ field }) => (
                    <RadioGroup
                      name={field.name}
                      value={field.value}
                      onValueChange={field.onChange}
                    >
                      <div className="flex items-center space-x-4">
                        <RadioGroupItem value="FIRST" id="r1" />
                        <Label htmlFor="r1" className="font-normal">
                          1st of the month
                        </Label>
                      </div>
                      <div className="flex items-center space-x-4">
                        <RadioGroupItem value="SUB_START_DAY" id="r2" />
                        <Label htmlFor="r2" className="font-normal">
                          Anniversary date of the subscription
                        </Label>
                      </div>
                    </RadioGroup>
                  )}
                />
                <InputFormField
                  name="netTerms"
                  label="Net Terms (days)"
                  type="number"
                  min={0}
                  placeholder={netTermsDefault ? String(netTermsDefault.netTerms) : undefined}
                  control={methods.control}
                  description={
                    netTermsDefault
                      ? `Leave empty for the default: ${formatDefaultNetTerms(netTermsDefault)}`
                      : 'Leave empty for the default terms'
                  }
                />
              </CardContent>
            </Card>
          </div>
        </PageSection>

        {/* Advanced Settings */}
        <PageSection
          header={{
            title: 'Advanced Settings',
            subtitle: 'Configure activation conditions and invoice details',
          }}
        >
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Activation & Lifecycle</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="space-y-2">
                  <SelectFormField
                    name="activationCondition"
                    label="Activation condition"
                    placeholder="Select when to activate"
                    control={methods.control}
                  >
                    <SelectItem value="ON_START">On Start Date</SelectItem>
                    <SelectItem
                      value="ON_CHECKOUT"
                      disabled={!isLoadingProviders && !hasOnlinePaymentProvider}
                    >
                      On Checkout
                      {!isLoadingProviders &&
                        !hasOnlinePaymentProvider &&
                        ' (requires a payment provider)'}
                    </SelectItem>
                    <SelectItem value="MANUAL">Manual Activation</SelectItem>
                  </SelectFormField>
                </div>
                <div className="space-y-2">
                  <SelectFormField
                    name="paymentMethodsType"
                    label="Payment methods"
                    placeholder="Select payment method type"
                    control={methods.control}
                    labelTooltip={
                      <TooltipProvider>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                          </TooltipTrigger>
                          <TooltipContent multiline className="max-w-96">
                            Online: Use card and/or direct debit payments via your payment
                            providers.
                            <br />
                            Bank Transfer: Invoice customers with bank transfer instructions.
                            <br />
                            External: Manage payment collection outside the system.
                          </TooltipContent>
                        </Tooltip>
                      </TooltipProvider>
                    }
                  >
                    <SelectItem value="online">Online (card / direct debit)</SelectItem>
                    <SelectItem
                      value="bankTransfer"
                      disabled={activationCondition === 'ON_CHECKOUT'}
                    >
                      Bank transfer
                      {activationCondition === 'ON_CHECKOUT' && ' (unavailable with Checkout)'}
                    </SelectItem>
                    <SelectItem value="external" disabled={activationCondition === 'ON_CHECKOUT'}>
                      External
                      {activationCondition === 'ON_CHECKOUT' && ' (unavailable with Checkout)'}
                    </SelectItem>
                  </SelectFormField>
                </div>
                <div className="grid grid-cols-2 gap-4">
                  <div className="space-y-2">
                    <div className="flex items-center gap-1">
                      <Label className="text-sm font-medium">Auto-advance invoices</Label>
                      <TooltipProvider>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                          </TooltipTrigger>
                          <TooltipContent multiline>
                            <p className="max-w-xs">
                              Automatically finalize draft invoices when generated. Disable to
                              manually review and finalize invoices.
                            </p>
                          </TooltipContent>
                        </Tooltip>
                      </TooltipProvider>
                    </div>
                    <GenericFormField
                      control={methods.control}
                      name="autoAdvanceInvoices"
                      render={({ field }) => (
                        <div className="flex items-center space-x-2">
                          <Switch
                            id="autoAdvanceInvoices"
                            checked={field.value}
                            onCheckedChange={field.onChange}
                          />
                          <Label htmlFor="autoAdvanceInvoices" className="font-normal text-sm">
                            {field.value ? 'Enabled' : 'Disabled'}
                          </Label>
                        </div>
                      )}
                    />
                  </div>
                  <div className="space-y-2">
                    <div className="flex items-center gap-1">
                      <Label className="text-sm font-medium">Charge automatically</Label>
                      <TooltipProvider>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                          </TooltipTrigger>
                          <TooltipContent multiline>
                            <p className="max-w-xs">
                              {!hasOnlinePaymentProvider
                                ? 'Requires a card or direct debit payment provider to be configured on the invoicing entity.'
                                : paymentMethodsType === 'bankTransfer' ||
                                    paymentMethodsType === 'external'
                                  ? 'Not available with Bank Transfer or External payment methods. Switch to Online to enable.'
                                  : 'Automatically try charging the customer when an invoice is finalized, if a payment method is configured.'}
                            </p>
                          </TooltipContent>
                        </Tooltip>
                      </TooltipProvider>
                    </div>
                    <GenericFormField
                      control={methods.control}
                      name="chargeAutomatically"
                      render={({ field }) => (
                        <div className="flex items-center space-x-2">
                          <Switch
                            id="chargeAutomatically"
                            checked={field.value}
                            onCheckedChange={field.onChange}
                            disabled={
                              (!isLoadingProviders && !hasOnlinePaymentProvider) ||
                              paymentMethodsType === 'bankTransfer' ||
                              paymentMethodsType === 'external'
                            }
                          />
                          <Label
                            htmlFor="chargeAutomatically"
                            className={`font-normal text-sm ${(!isLoadingProviders && !hasOnlinePaymentProvider) || paymentMethodsType === 'bankTransfer' || paymentMethodsType === 'external' ? 'text-muted-foreground' : ''}`}
                          >
                            {field.value ? 'Enabled' : 'Disabled'}
                            {!isLoadingProviders && !hasOnlinePaymentProvider
                              ? ' (requires a payment provider)'
                              : (paymentMethodsType === 'bankTransfer' ||
                                    paymentMethodsType === 'external') &&
                                  field.value
                                ? ' (disabled by payment method)'
                                : ''}
                          </Label>
                        </div>
                      )}
                    />
                  </div>
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-base">Invoice Customization</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <TextareaFormField
                  name="invoiceMemo"
                  label="Invoice Memo"
                  placeholder="Custom note for invoices..."
                  control={methods.control}
                  rows={3}
                />
                {/* <InputFormField
                  name="invoiceThreshold"
                  label="Invoice Threshold"
                  placeholder="100.00"
                  control={methods.control}
                /> */}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-base">Metadata</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <InputFormField
                  name="purchaseOrder"
                  label="Purchase Order"
                  placeholder="Enter purchase order..."
                  control={methods.control}
                />
                <CustomPropertiesFields
                  control={methods.control}
                  entityType={CustomPropertyEntityType.SUBSCRIPTION}
                  title={null}
                />
              </CardContent>
            </Card>
          </div>
        </PageSection>

        {!isFreePlan && planCurrency && hasSlot('subscriptionCreate.minimumCommitment') && (
          <PageSection
            header={{
              title: 'Contract terms',
              subtitle:
                'Inherited from the plan by default. You can override the minimum (preserved across plan changes) or drop it for this subscription.',
            }}
          >
            <Card>
              <CardContent className="pt-6">
                <Slot
                  name="subscriptionCreate.minimumCommitment"
                  currency={planCurrency}
                  allowDrop
                  allowPending
                  components={scopeOptionsWithPending([
                    // Effective product set for this subscription: plan components minus removed,
                    // plus ad-hoc extras (so the scope reflects what will actually be billed).
                    // Inline new-product extras (no id yet) show as disabled "pending" entries.
                    ...(minimumComponentsQuery.data?.components ?? []).filter(
                      c => !state.components.removed.includes(c.id)
                    ),
                    ...state.components.extra.map(e => ({ name: e.name, productId: e.productId })),
                  ])}
                  planDefault={planQuery.data?.plan?.version?.minimumCommitment}
                  override={state.minimumOverride}
                  dropped={!!state.dropMinimum}
                  onChange={({ override, dropped }) =>
                    setState({ ...state, minimumOverride: override, dropMinimum: dropped })
                  }
                />
              </CardContent>
            </Card>
          </PageSection>
        )}

        <div className="flex gap-2 justify-end">
          <Button type="button" onClick={previousStep} variant="secondary">
            Back
          </Button>
          <Button type="submit">Next: Summary</Button>
        </div>
      </form>
    </Form>
  )
}

const schema = z
  .object({
    fromDate: z.date(),
    toDate: z.date().optional(),
    billingDay: z.enum(['FIRST', 'SUB_START_DAY']).default('SUB_START_DAY'),
    startOfferMode: z.enum(['plan', 'none', 'catalog']).default('plan'),
    startOfferId: z.string().optional(),
    trialMode: z.enum(['plan', 'none', 'custom']).default('plan'),
    trialDuration: z.number().int().min(1).max(3650).optional(),
    trialUntil: z.date().optional(),
    trialFeaturesOf: z.string().optional(),
    activationCondition: z.enum(['ON_START', 'ON_CHECKOUT', 'MANUAL']),
    paymentMethodsType: z.enum(['online', 'bankTransfer', 'external']).default('online'),
    netTerms: z.preprocess(
      value => (value === '' || value === null ? undefined : value),
      z.number().int('Enter a whole number of days').min(0).optional()
    ),
    invoiceMemo: z.string().optional(),
    invoiceThreshold: z.string().optional(),
    purchaseOrder: z.string().optional(),
    autoAdvanceInvoices: z.boolean().default(true),
    chargeAutomatically: z.boolean().default(true),
    skipPastInvoices: z.boolean().default(false),
    customProperties: z.record(z.string(), z.unknown()).optional(),
  })
  .refine(data => data.trialMode !== 'custom' || !!data.trialDuration || !!data.trialUntil, {
    message: 'Enter days or an end date',
    path: ['trialDuration'],
  })
  .refine(data => data.startOfferMode !== 'catalog' || !!data.startOfferId, {
    message: 'Pick an offer',
    path: ['startOfferId'],
  })
  .refine(data => !data.toDate || data.toDate > data.fromDate, {
    message: 'Must be greater than the start date',
    path: ['toDate'],
  })
  .refine(
    data => {
      // OnCheckout requires online payment methods
      if (data.activationCondition === 'ON_CHECKOUT' && data.paymentMethodsType !== 'online') {
        return false
      }
      return true
    },
    {
      message: 'OnCheckout activation requires online payment methods',
      path: ['activationCondition'],
    }
  )
  .refine(
    data => {
      // ChargeAutomatically doesn't make sense with Bank or External payment methods
      if (
        data.chargeAutomatically &&
        (data.paymentMethodsType === 'bankTransfer' || data.paymentMethodsType === 'external')
      ) {
        return false
      }
      return true
    },
    {
      message: 'Automatic charging requires online payment methods',
      path: ['chargeAutomatically'],
    }
  )
  .refine(
    data => {
      // skipPastInvoices only works with ON_START activation
      if (data.skipPastInvoices && data.activationCondition !== 'ON_START') {
        return false
      }
      return true
    },
    {
      message: 'Skip past invoices requires "On Start Date" activation condition',
      path: ['skipPastInvoices'],
    }
  )
