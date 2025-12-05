import { PartialMessage } from '@bufbuild/protobuf'
import { disableQuery, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  DateFormField,
  DatePicker,
  Form,
  GenericFormField,
  Input,
  InputFormField,
  Label,
  SelectFormField,
  SelectItem,
  Switch,
  TextareaFormField,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { Eye, Gift, InfoIcon, Plus, Save, Search, Tag, Trash2, X } from 'lucide-react'
import { customAlphabet } from 'nanoid'
import { useEffect, useState } from 'react'
import { useFieldArray } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { z } from 'zod'

import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { SubscribablePlanVersionSelect } from '@/features/plans/SubscribablePlanVersionSelect'
import { QuotePriceComponentsWrapper } from '@/features/quotes/QuotePriceComponentsWrapper'
import { QuoteView } from '@/features/quotes/QuoteView'
import {
  mapExtraComponentToSubscriptionComponent,
  mapOverrideComponentToSubscriptionComponent,
  PriceComponentsState,
} from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import { useBasePath } from '@/hooks/useBasePath'
import { useCurrency } from '@/hooks/useCurrency'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { mapDatev2 } from '@/lib/mapping'
import { ComponentParameterization } from '@/pages/tenants/subscription/create/state'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import {
  getInvoicingEntity,
  getInvoicingEntityProviders,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import {
  CreateQuoteCoupon,
  CreateQuoteCoupons,
  CreateQuote as CreateQuoteData,
  DetailedQuote,
  PaymentStrategy,
  Quote,
  QuoteComponent,
} from '@/rpc/api/quotes/v1/models_pb'
import { createQuote } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { CreateQuoteRequest } from '@/rpc/api/quotes/v1/quotes_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import {
  ActivationCondition,
  CreateSubscriptionAddOn,
  CreateSubscriptionAddOns,
  CreateSubscriptionComponents,
  SubscriptionComponentNewInternal,
  SubscriptionFee,
  SubscriptionFee_CapacitySubscriptionFee,
  SubscriptionFee_ExtraRecurringSubscriptionFee,
  SubscriptionFee_OneTimeSubscriptionFee,
  SubscriptionFee_RateSubscriptionFee,
  SubscriptionFee_SlotSubscriptionFee,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'

const recipientSchema = z.object({
  name: z.string().min(1, 'Recipient name is required'),
  email: z.string().email('Valid email is required'),
})

const createQuoteSchema = z.object({
  quote_number: z.string().min(1, 'Quote number is required'),
  customer_id: z.string().min(1, 'Customer is required'),
  plan_version_id: z.string().min(1, 'Plan is required'),
  currency: z.string().min(1, 'Currency is required'),
  start_date: z.date().optional(),
  billing_start_date: z.date().optional(),
  end_date: z.date().optional(),
  trial_duration: z.number().min(0).optional(),
  billing_day_anchor: z.preprocess(val => {
    if (val === '') return undefined
    return val
  }, z.number().min(1).max(31).optional()),
  expires_at: z.date().optional(),
  valid_until: z.date().optional(),
  internal_notes: z.string().optional(),
  overview: z.string().optional(),
  terms_and_services: z.string().optional(),
  net_terms: z.number().min(0).default(30),
  recipients: z.array(recipientSchema).min(1, 'At least one recipient is required'),
  // Advanced settings
  activation_condition: z.enum(['ON_START', 'ON_CHECKOUT', 'MANUAL']).default('ON_START'),
  payment_strategy: z.enum(['AUTO', 'BANK', 'EXTERNAL']).default('AUTO'),
  auto_advance_invoices: z.boolean().default(true),
  charge_automatically: z.boolean().default(true),
  invoice_memo: z.string().optional(),
  create_subscription_on_acceptance: z.boolean().default(false),
})

type CreateQuoteFormData = z.infer<typeof createQuoteSchema>

const nanoid = customAlphabet('ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789', 10)

// Helper functions for activation condition and payment strategy
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

const paymentStrategyFromString = (strategy: 'AUTO' | 'BANK' | 'EXTERNAL'): PaymentStrategy => {
  switch (strategy) {
    case 'AUTO':
      return PaymentStrategy.AUTO
    case 'BANK':
      return PaymentStrategy.BANK
    case 'EXTERNAL':
      return PaymentStrategy.EXTERNAL
    default:
      return PaymentStrategy.AUTO
  }
}

export const CreateQuote = () => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  const [priceComponentsState, setPriceComponentsState] = useState<PriceComponentsState>({
    components: {
      removed: [],
      parameterized: [],
      overridden: [],
      extra: [],
    },
  })

  // Add-ons and coupons state
  const [selectedAddOns, setSelectedAddOns] = useState<{ addOnId: string }[]>([])
  const [selectedCoupons, setSelectedCoupons] = useState<{ couponId: string }[]>([])
  const [addOnSearch, setAddOnSearch] = useState('')
  const [couponSearch, setCouponSearch] = useState('')

  const createQuoteMutation = useMutation(createQuote, {
    onSuccess: data => {
      navigate(`${basePath}/quotes/${data.quote?.quote?.id}`)
    },
    onError: error => {
      console.error('Failed to create quote:', error)
    },
  })

  const { currency } = useCurrency()

  const methods = useZodForm({
    schema: createQuoteSchema,
    defaultValues: {
      quote_number: `Q-${new Date().toISOString().slice(0, 10).replace(/-/g, '')}-${nanoid(5)}`,
      customer_id: '',
      plan_version_id: '',
      currency: currency,
      net_terms: 30,
      recipients: [{ name: '', email: '' }],
      // Advanced settings defaults
      activation_condition: 'ON_START',
      payment_strategy: 'AUTO',
      auto_advance_invoices: true,
      charge_automatically: true,
      create_subscription_on_acceptance: false,
    },
  })

  const {
    fields: recipientFields,
    append: appendRecipient,
    remove: removeRecipient,
  } = useFieldArray({
    control: methods.control,
    name: 'recipients',
  })

  const [previewMode, setPreviewMode] = useState(false)
  const [pricingValidation, setPricingValidation] = useState({
    isValid: false,
    errors: [] as string[],
  })
  const [customerId, planVersionId] = methods.watch(['customer_id', 'plan_version_id'])

  const customerQuery = useQuery(
    getCustomerById,
    { id: customerId ?? '' },
    { enabled: Boolean(customerId) }
  )

  useEffect(() => {
    if (customerQuery.data?.customer?.billingEmail) {
      const existing = methods.getValues('recipients')

      if (existing.length === 1 && existing[0].email === '') {
        methods.setValue('recipients', [
          {
            name: customerQuery.data.customer.name,
            email: customerQuery.data.customer.billingEmail,
          },
        ])
      }
    }
  }, [customerQuery.data?.customer])

  const planQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: planVersionId ?? '' },
    { enabled: Boolean(planVersionId) }
  )

  useEffect(() => {
    if (planQuery.data?.plan?.version?.currency) {
      methods.setValue('currency', planQuery.data.plan.version.currency)
    }
  }, [planQuery.data?.plan])

  const invoicingEntityQuery = useQuery(
    getInvoicingEntity,
    {
      id: customerQuery.data?.customer?.invoicingEntityId || '',
    },
    { enabled: Boolean(customerQuery.data?.customer?.invoicingEntityId) }
  )

  useEffect(() => {
    if (!planQuery.data?.plan?.version?.currency && currency) {
      methods.setValue('currency', currency)
    }
  }, [currency, planQuery.data?.plan?.version?.currency])

  const priceComponentsQuery = useQuery(
    listPriceComponents,
    {
      planVersionId: planVersionId ?? '',
    },
    { enabled: Boolean(planVersionId) }
  )

  // Add-ons and coupons queries
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

  // Invoicing entity providers query for payment provider check
  const invoicingEntityId = customerQuery.data?.customer?.invoicingEntityId
  const providersQuery = useQuery(
    getInvoicingEntityProviders,
    invoicingEntityId ? { id: invoicingEntityId } : disableQuery
  )

  const hasOnlinePaymentProvider =
    !!providersQuery.data?.cardProvider || !!providersQuery.data?.directDebitProvider
  const isLoadingProviders = customerQuery.isLoading || providersQuery.isLoading

  // Watch advanced settings for cross-validation
  const [activationCondition, paymentStrategy, chargeAutomatically] = methods.watch([
    'activation_condition',
    'payment_strategy',
    'charge_automatically',
  ])

  // Auto-disable chargeAutomatically and reset activationCondition when no online provider
  useEffect(() => {
    if (!isLoadingProviders && !hasOnlinePaymentProvider) {
      const currentActivation = methods.getValues('activation_condition')
      const currentCharge = methods.getValues('charge_automatically')

      if (currentActivation === 'ON_CHECKOUT') {
        methods.setValue('activation_condition', 'ON_START')
      }
      if (currentCharge) {
        methods.setValue('charge_automatically', false)
      }
    }
  }, [hasOnlinePaymentProvider, isLoadingProviders, methods])

  // Auto-set payment strategy to Auto when OnCheckout is selected
  useEffect(() => {
    if (activationCondition === 'ON_CHECKOUT' && paymentStrategy !== 'AUTO') {
      methods.setValue('payment_strategy', 'AUTO')
    }
  }, [activationCondition, paymentStrategy, methods])

  // Auto-disable chargeAutomatically when Bank or External payment strategy is selected
  useEffect(() => {
    if ((paymentStrategy === 'BANK' || paymentStrategy === 'EXTERNAL') && chargeAutomatically) {
      methods.setValue('charge_automatically', false)
    }
  }, [paymentStrategy, chargeAutomatically, methods])

  const onSubmit = async (data: CreateQuoteFormData) => {
    try {
      const configuredComponents = extractConfiguredComponents(priceComponentsState)

      const subscriptionComponents = new CreateSubscriptionComponents({
        parameterizedComponents: configuredComponents.parameterizedComponents,
        overriddenComponents: configuredComponents.overriddenComponents,
        extraComponents: configuredComponents.extraComponents,
        removeComponents: configuredComponents.removedComponentIds,
      })

      // Build add-ons
      const addOns = new CreateSubscriptionAddOns({
        addOns: selectedAddOns.map(a => new CreateSubscriptionAddOn({ addOnId: a.addOnId })),
      })

      // Build coupons
      const coupons = new CreateQuoteCoupons({
        coupons: selectedCoupons.map(c => new CreateQuoteCoupon({ couponId: c.couponId })),
      })

      const createQuoteData = new CreateQuoteData({
        quoteNumber: data.quote_number,
        planVersionId: data.plan_version_id,
        customerId: data.customer_id,
        currency: data.currency ?? planQuery.data?.plan?.version?.currency,
        startDate: data.start_date ? mapDatev2(data.start_date) : undefined,
        billingStartDate: data.billing_start_date ? mapDatev2(data.billing_start_date) : undefined,
        endDate: data.end_date ? mapDatev2(data.end_date) : undefined,
        trialDuration: data.trial_duration,
        billingDayAnchor: data.billing_day_anchor,
        expiresAt: data.expires_at ? mapDatev2(data.expires_at) : undefined,
        validUntil: data.valid_until ? mapDatev2(data.valid_until) : undefined,
        internalNotes: data.internal_notes,
        overview: data.overview,
        termsAndServices: data.terms_and_services,
        netTerms: data.net_terms,
        activationCondition: activationConditionFromString(data.activation_condition),
        paymentStrategy: paymentStrategyFromString(data.payment_strategy),
        autoAdvanceInvoices: data.auto_advance_invoices,
        chargeAutomatically: data.charge_automatically,
        invoiceMemo: data.invoice_memo,
        createSubscriptionOnAcceptance: data.create_subscription_on_acceptance,
        attachments: [], // TODO: Add attachment support
        recipients: data.recipients.map(r => ({ name: r.name, email: r.email })),
        components: subscriptionComponents,
        addOns: addOns,
        coupons: coupons,
      })

      const request = new CreateQuoteRequest({
        quote: createQuoteData,
      })

      const response = await createQuoteMutation.mutateAsync(request)

      if (response.quote?.quote?.id) {
        navigate(`${basePath}/quotes/${response.quote.quote.id}`)
      }
    } catch (error) {
      console.error('Failed to create quote:', error)
    }
  }

  const addRecipient = () => {
    appendRecipient({ name: '', email: '' })
  }

  const removeRecipientAt = (index: number) => {
    if (recipientFields.length > 1) {
      removeRecipient(index)
    }
  }

  const createPreviewQuote = (data: CreateQuoteFormData): DetailedQuote => {
    const quote = new Quote({
      id: 'preview-quote',
      quoteNumber: data.quote_number,
      planVersionId: data.plan_version_id,
      customerId: data.customer_id,
      currency: data.currency ?? planQuery.data?.plan?.version?.currency,
      startDate: data.start_date ? mapDatev2(data.start_date) : mapDatev2(new Date()),
      billingStartDate: data.billing_start_date ? mapDatev2(data.billing_start_date) : undefined,
      endDate: data.end_date ? mapDatev2(data.end_date) : undefined,
      trialDuration: data.trial_duration,
      billingDayAnchor: data.billing_day_anchor,
      expiresAt: data.expires_at ? mapDatev2(data.expires_at) : undefined,
      validUntil: data.valid_until ? mapDatev2(data.valid_until) : undefined,
      internalNotes: data.internal_notes,
      overview: data.overview,
      termsAndServices: data.terms_and_services,
      netTerms: data.net_terms,
      recipients: data.recipients.map(r => ({ name: r.name, email: r.email })),
      createdAt: new Date().toISOString(),
    })

    const components: PartialMessage<QuoteComponent>[] = getPreviewPricingComponents()

    return new DetailedQuote({
      quote,
      components,
      customer: customerQuery.data?.customer,
      invoicingEntity: invoicingEntityQuery.data?.entity,
    })
  }

  const getPreviewPricingComponents = () => {
    const priceComponentsData = priceComponentsQuery.data?.components || []

    console.log('getPreviewPricingComponents priceComponentsData', priceComponentsData)

    const configuredComponents = extractConfiguredComponents(priceComponentsState)

    const mapParameterized = (c: ComponentParameterization) => {
      const pc = priceComponentsData.find(pc => pc.id === c.componentId)
      if (!pc) {
        return null
      }
      return mapParameterizedComponentToSubscriptionComponent(c, pc)
    }

    // Create default components for plan components that haven't been configured
    const defaultPlanComponents = priceComponentsData
      .filter(
        pc =>
          !configuredComponents.removedComponentIds.includes(pc.id) &&
          !configuredComponents.parameterizedComponents.some(c => c.componentId === pc.id) &&
          !configuredComponents.overriddenComponents.some(c => c.componentId === pc.id)
      )
      .map(pc => mapDefaultComponentToSubscriptionComponent(pc))

    console.log('defaultPlanComponents', defaultPlanComponents)

    const allSubscriptionComponents = [
      ...defaultPlanComponents,
      ...configuredComponents.parameterizedComponents
        .map(mapParameterized)
        .filter((c): c is SubscriptionComponentNewInternal => c !== null),
      ...configuredComponents.overriddenComponents.map(c => c.component),
      ...configuredComponents.extraComponents.map(c => c.component),
    ]

    return allSubscriptionComponents.map(comp => ({
      id: comp.priceComponentId || comp.name,
      name: comp.name,
      isOverride: !comp.priceComponentId,
      period: comp.period || SubscriptionFeeBillingPeriod.MONTHLY,
      fee: comp.fee,
    }))
  }

  if (previewMode) {
    const formData = methods.getValues()
    const previewQuote = createPreviewQuote(formData)
    const previewComponents = getPreviewPricingComponents()

    return (
      <div className="space-y-6">
        <div className="flex justify-between items-center">
          <h1 className="text-2xl font-semibold">Quote Preview</h1>
          <div className="flex gap-2">
            <Button variant="outline" onClick={() => setPreviewMode(false)}>
              Back to Edit
            </Button>
            <Button
              onClick={methods.handleSubmit(onSubmit)}
              disabled={createQuoteMutation.isPending}
            >
              <Save className="w-4 h-4 mr-2" />
              {createQuoteMutation.isPending ? 'Creating...' : 'Create Quote'}
            </Button>
          </div>
        </div>
        <QuoteView quote={previewQuote} mode="preview" subscriptionComponents={previewComponents} />
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <div className="flex justify-between items-center">
        <div>
          <h1 className="text-2xl font-semibold">Create Quote</h1>
          <p className="text-muted-foreground">Build a professional quote for your customer</p>
        </div>
        <div className="flex gap-2">
          <Button
            variant="outline"
            onClick={() => setPreviewMode(true)}
            disabled={
              !customerId ||
              !planVersionId ||
              !pricingValidation.isValid ||
              priceComponentsQuery.isLoading
            }
          >
            <Eye className="w-4 h-4 mr-2" />
            Preview
          </Button>
          <Button
            onClick={methods.handleSubmit(onSubmit)}
            disabled={
              createQuoteMutation.isPending ||
              !customerId ||
              !planVersionId ||
              !pricingValidation.isValid
            }
          >
            <Save className="w-4 h-4 mr-2" />
            {createQuoteMutation.isPending ? 'Creating...' : 'Create Quote'}
          </Button>
        </div>
      </div>

      <Form {...methods}>
        <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-6">
          <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
            {/* Main Form */}
            <div className="lg:col-span-2 space-y-6 pb-4">
              {/* Customer & Plan Selection */}
              <Card>
                <CardHeader>
                  <CardTitle>Customer & Plan</CardTitle>
                  <CardDescription>Select the customer and plan for this quote</CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div className="grid grid-cols-1 gap-4">
                    <GenericFormField
                      control={methods.control}
                      layout="horizontal"
                      label="Customer"
                      name="customer_id"
                      render={({ field }) => (
                        <CustomerSelect value={field.value} onChange={field.onChange} />
                      )}
                    />

                    <GenericFormField
                      control={methods.control}
                      layout="horizontal"
                      label="Plan"
                      name="plan_version_id"
                      render={({ field }) => (
                        <SubscribablePlanVersionSelect
                          value={field.value}
                          onChange={field.onChange}
                        />
                      )}
                    />

                    <InputFormField
                      name="currency"
                      label="Currency"
                      control={methods.control}
                      disabled
                      className="w-[180px]"
                      layout="horizontal"
                    />
                  </div>
                </CardContent>
              </Card>

              {/* Quote Details */}
              <Card>
                <CardHeader>
                  <CardTitle>Quote Details</CardTitle>
                  <CardDescription>Basic information about this quote</CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
                    <InputFormField
                      name="quote_number"
                      label="Quote Number"
                      control={methods.control}
                      placeholder="Q-12345"
                      description="Unique identifier for this quote"
                      layout="vertical"
                    />
                    <DateFormField
                      name="expires_at"
                      label="Quote Expires On (Optional)"
                      control={methods.control}
                      layout="vertical"
                      clearable
                    />
                  </div>

                  <TextareaFormField
                    name="overview"
                    label="Overview"
                    control={methods.control}
                    placeholder="Brief overview of the quote (visible to customer)"
                    layout="vertical"
                  />

                  <TextareaFormField
                    name="terms_and_services"
                    label="Terms & Services"
                    control={methods.control}
                    placeholder="Terms and conditions for this quote"
                    layout="vertical"
                  />

                  <TextareaFormField
                    name="internal_notes"
                    label="Internal Notes"
                    control={methods.control}
                    placeholder="Internal notes (not visible to customer)"
                    layout="vertical"
                  />
                </CardContent>
              </Card>

              {/* Subscription Pricing */}
              {planVersionId && customerId && (
                <Card>
                  <CardHeader>
                    <CardTitle>Subscription Pricing</CardTitle>
                    <CardDescription>
                      Configure the subscription components and pricing for this quote
                    </CardDescription>
                  </CardHeader>
                  <CardContent className="space-y-4">
                    <QuotePriceComponentsWrapper
                      planVersionId={planVersionId}
                      customerId={customerId}
                      onValidationChange={(isValid, errors) =>
                        setPricingValidation({ isValid, errors })
                      }
                      onStateChange={setPriceComponentsState}
                      initialState={priceComponentsState}
                    />
                    {!pricingValidation.isValid && pricingValidation.errors.length > 0 && (
                      <div className="bg-yellow-50 border border-yellow-200 p-3 rounded-lg">
                        <p className="text-sm font-medium text-yellow-800">
                          Configuration Required:
                        </p>
                        <ul className="text-sm text-yellow-700 mt-1 list-disc list-inside">
                          {pricingValidation.errors.map((error, index) => (
                            <li key={index}>{error}</li>
                          ))}
                        </ul>
                      </div>
                    )}
                  </CardContent>
                </Card>
              )}

              {/* Add-ons & Coupons */}
              {planVersionId && customerId && (
                <Card>
                  <CardHeader>
                    <CardTitle>Add-ons & Discounts</CardTitle>
                    <CardDescription>Optional add-ons and promotional coupons</CardDescription>
                  </CardHeader>
                  <CardContent className="space-y-6">
                    {/* Add-ons Section */}
                    <div className="space-y-4">
                      <div>
                        <h3 className="text-sm font-medium mb-3 flex items-center gap-2">
                          <Plus className="h-4 w-4 text-green-500" />
                          Add-ons
                          <Badge variant="outline" size="sm">
                            {selectedAddOns.length} selected
                          </Badge>
                        </h3>

                        {selectedAddOns.length > 0 ? (
                          <div className="grid gap-2 mb-3">
                            {selectedAddOns.map(addon => {
                              const addOnData = availableAddOns.find(a => a.id === addon.addOnId)
                              return (
                                <Card
                                  key={addon.addOnId}
                                  className="border-green-200 bg-green-50/30"
                                >
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
                                        setSelectedAddOns(prev =>
                                          prev.filter(a => a.addOnId !== addon.addOnId)
                                        )
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
                              const isSelected = selectedAddOns.some(a => a.addOnId === addOn.id)
                              return (
                                <Button
                                  key={addOn.id}
                                  type="button"
                                  variant="ghost"
                                  size="sm"
                                  className="w-full justify-start text-sm"
                                  disabled={isSelected}
                                  onClick={() => {
                                    setSelectedAddOns(prev => [...prev, { addOnId: addOn.id }])
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
                            {selectedCoupons.length} applied
                          </Badge>
                        </h3>

                        {selectedCoupons.length > 0 ? (
                          <div className="grid gap-2 mb-3">
                            {selectedCoupons.map(coupon => {
                              const couponData = availableCoupons.find(
                                c => c.id === coupon.couponId
                              )
                              return (
                                <Card
                                  key={coupon.couponId}
                                  className="bg-card text-card-foreground"
                                >
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
                                        setSelectedCoupons(prev =>
                                          prev.filter(c => c.couponId !== coupon.couponId)
                                        )
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
                              const isSelected = selectedCoupons.some(c => c.couponId === coupon.id)
                              return (
                                <Button
                                  key={coupon.id}
                                  type="button"
                                  variant="ghost"
                                  size="sm"
                                  className="w-full justify-start text-sm"
                                  disabled={isSelected}
                                  onClick={() => {
                                    setSelectedCoupons(prev => [...prev, { couponId: coupon.id }])
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
                  </CardContent>
                </Card>
              )}

              {/* Subscription Settings */}
              <Card>
                <CardHeader>
                  <CardTitle>Subscription Settings</CardTitle>
                  <CardDescription>
                    Configure subscription dates, activation, and payment options
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-6">
                  {/* Subscription Creation Toggle */}
                  <div className="flex items-center justify-between p-4 border rounded-lg bg-muted/30">
                    <div className="flex items-center gap-2">
                      <Label className="text-sm font-medium">
                        Create subscription on acceptance
                      </Label>
                      <TooltipProvider delayDuration={100}>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                          </TooltipTrigger>
                          <TooltipContent>
                            <p className="max-w-xs">
                              When enabled, a subscription will be automatically created when the
                              quote is signed by all recipients.
                            </p>
                          </TooltipContent>
                        </Tooltip>
                      </TooltipProvider>
                    </div>
                    <GenericFormField
                      control={methods.control}
                      name="create_subscription_on_acceptance"
                      render={({ field }) => (
                        <div className="flex items-center space-x-2">
                          <Switch
                            id="createSubscriptionOnAcceptance"
                            checked={field.value}
                            onCheckedChange={field.onChange}
                          />
                          <Label
                            htmlFor="createSubscriptionOnAcceptance"
                            className="font-normal text-sm"
                          >
                            {field.value ? 'Enabled' : 'Disabled'}
                          </Label>
                        </div>
                      )}
                    />
                  </div>

                  {/* Subscription Dates */}
                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
                    <div className="space-y-4">
                      <h4 className="text-sm font-medium">Subscription Dates</h4>
                      <GenericFormField
                        control={methods.control}
                        layout="vertical"
                        label="Start Date (Optional)"
                        name="start_date"
                        description="Leave empty for subscription creation date"
                        render={({ field }) => (
                          <DatePicker
                            mode="single"
                            captionLayout="dropdown"
                            className="min-w-[12em]"
                            placeholder="Dynamic"
                            date={field.value}
                            onSelect={field.onChange}
                            clearable
                          />
                        )}
                      />
                      <GenericFormField
                        control={methods.control}
                        layout="vertical"
                        label="End Date (Optional)"
                        name="end_date"
                        render={({ field }) => (
                          <DatePicker
                            mode="single"
                            captionLayout="dropdown"
                            className="min-w-[12em]"
                            placeholder="No end date"
                            date={field.value}
                            onSelect={field.onChange}
                            clearable
                          />
                        )}
                      />
                    </div>
                    <div className="space-y-4">
                      <h4 className="text-sm font-medium">Billing Configuration</h4>
                      <InputFormField
                        name="billing_day_anchor"
                        label="Billing Day"
                        control={methods.control}
                        type="number"
                        placeholder="Anniversary"
                        description="Day of month for billing (1-31). Leave empty for anniversary"
                        layout="vertical"
                      />
                      <InputFormField
                        name="net_terms"
                        label="Net Terms"
                        control={methods.control}
                        type="number"
                        placeholder="30"
                        description="Payment terms in days"
                        layout="vertical"
                      />
                    </div>
                  </div>

                  <div className="border-t pt-4 grid grid-cols-1 lg:grid-cols-2 gap-6">
                    {/* Activation & Lifecycle */}
                    <div className="space-y-4">
                      <h4 className="text-sm font-medium">Activation & Lifecycle</h4>
                      <SelectFormField
                        name="activation_condition"
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

                      <SelectFormField
                        name="payment_strategy"
                        label="Payment methods strategy"
                        placeholder="Select payment strategy"
                        control={methods.control}
                        labelTooltip={
                          <TooltipProvider delayDuration={100}>
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                              </TooltipTrigger>
                              <TooltipContent className="max-w-96">
                                Default strategy will try configured online payment providers first,
                                then fall back to offline methods. <br />
                                Bank Transfer and External options restrict payment methods
                                accordingly.
                              </TooltipContent>
                            </Tooltip>
                          </TooltipProvider>
                        }
                      >
                        <SelectItem value="AUTO">Default</SelectItem>
                        <SelectItem value="BANK" disabled={activationCondition === 'ON_CHECKOUT'}>
                          Bank transfer
                          {activationCondition === 'ON_CHECKOUT' && ' (unavailable with Checkout)'}
                        </SelectItem>
                        <SelectItem
                          value="EXTERNAL"
                          disabled={activationCondition === 'ON_CHECKOUT'}
                        >
                          External
                          {activationCondition === 'ON_CHECKOUT' && ' (unavailable with Checkout)'}
                        </SelectItem>
                      </SelectFormField>
                    </div>

                    {/* Invoice Options */}
                    <div className="space-y-4">
                      <h4 className="text-sm font-medium">Invoicing Options</h4>
                      <div className="grid grid-cols-2 gap-4">
                        <div className="space-y-2">
                          <div className="flex items-center gap-1">
                            <Label className="text-sm font-medium">Auto-advance invoices</Label>
                            <TooltipProvider delayDuration={100}>
                              <Tooltip>
                                <TooltipTrigger asChild>
                                  <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                                </TooltipTrigger>
                                <TooltipContent>
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
                            name="auto_advance_invoices"
                            render={({ field }) => (
                              <div className="flex items-center space-x-2">
                                <Switch
                                  id="autoAdvanceInvoices"
                                  checked={field.value}
                                  onCheckedChange={field.onChange}
                                />
                                <Label
                                  htmlFor="autoAdvanceInvoices"
                                  className="font-normal text-sm"
                                >
                                  {field.value ? 'Enabled' : 'Disabled'}
                                </Label>
                              </div>
                            )}
                          />
                        </div>
                        <div className="space-y-2">
                          <div className="flex items-center gap-1">
                            <Label className="text-sm font-medium">Charge automatically</Label>
                            <TooltipProvider delayDuration={100}>
                              <Tooltip>
                                <TooltipTrigger asChild>
                                  <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                                </TooltipTrigger>
                                <TooltipContent>
                                  <p className="max-w-xs">
                                    {!hasOnlinePaymentProvider
                                      ? 'Requires a card or direct debit payment provider to be configured on the invoicing entity.'
                                      : paymentStrategy === 'BANK' || paymentStrategy === 'EXTERNAL'
                                        ? 'Not available with Bank or External payment strategies. Switch to Default payment strategy to enable.'
                                        : 'Automatically try charging the customer when an invoice is finalized, if a payment method is configured.'}
                                  </p>
                                </TooltipContent>
                              </Tooltip>
                            </TooltipProvider>
                          </div>
                          <GenericFormField
                            control={methods.control}
                            name="charge_automatically"
                            render={({ field }) => (
                              <div className="flex items-center space-x-2">
                                <Switch
                                  id="chargeAutomatically"
                                  checked={field.value}
                                  onCheckedChange={field.onChange}
                                  disabled={
                                    (!isLoadingProviders && !hasOnlinePaymentProvider) ||
                                    paymentStrategy === 'BANK' ||
                                    paymentStrategy === 'EXTERNAL'
                                  }
                                />
                                <Label
                                  htmlFor="chargeAutomatically"
                                  className={`font-normal text-sm ${(!isLoadingProviders && !hasOnlinePaymentProvider) || paymentStrategy === 'BANK' || paymentStrategy === 'EXTERNAL' ? 'text-muted-foreground' : ''}`}
                                >
                                  {field.value ? 'Enabled' : 'Disabled'}
                                </Label>
                              </div>
                            )}
                          />
                        </div>
                      </div>
                      <TextareaFormField
                        name="invoice_memo"
                        label="Invoice Memo"
                        control={methods.control}
                        placeholder="Custom note for invoices..."
                        layout="vertical"
                        rows={2}
                      />
                    </div>
                  </div>
                </CardContent>
              </Card>

              {/* Recipients */}
              <Card>
                <CardHeader>
                  <CardTitle>Recipients</CardTitle>
                  <CardDescription>Who should receive this quote</CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  {recipientFields.map((field, index) => (
                    <div key={field.id} className="p-4 border rounded-lg space-y-4">
                      <div className="flex justify-between items-center">
                        <h4 className="font-medium">Recipient {index + 1}</h4>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          onClick={() => removeRecipientAt(index)}
                          disabled={recipientFields.length === 1}
                        >
                          <Trash2 className="w-4 h-4" />
                        </Button>
                      </div>

                      <div className="grid grid-cols-2 gap-4">
                        <InputFormField
                          name={`recipients.${index}.name`}
                          label="Name"
                          control={methods.control}
                          placeholder="John Doe"
                          layout="vertical"
                        />

                        <InputFormField
                          name={`recipients.${index}.email`}
                          label="Email"
                          control={methods.control}
                          type="email"
                          placeholder="john@company.com"
                          layout="vertical"
                        />
                      </div>
                    </div>
                  ))}

                  <Button type="button" variant="outline" onClick={addRecipient} className="w-full">
                    <Plus className="w-4 h-4 mr-2" />
                    Add Recipient
                  </Button>
                </CardContent>
              </Card>

              <div>
                <Button
                  className="w-full"
                  onClick={methods.handleSubmit(onSubmit)}
                  disabled={createQuoteMutation.isPending}
                >
                  <Save className="w-4 h-4 mr-2" />
                  {createQuoteMutation.isPending ? 'Creating...' : 'Create Quote'}
                </Button>
              </div>
            </div>

            {/* Summary Sidebar */}
            <div className="space-y-6">
              <Card>
                <CardHeader>
                  <CardTitle>Quote Summary</CardTitle>
                </CardHeader>
                <CardContent className="space-y-4">
                  <div className="space-y-2 text-sm">
                    <div className="flex justify-between">
                      <span>Customer:</span>
                      <span className="text-muted-foreground">
                        {customerId ? 'Selected' : 'Not selected'}
                      </span>
                    </div>
                    <div className="flex justify-between">
                      <span>Plan:</span>
                      <span className="text-muted-foreground">
                        {planVersionId ? 'Selected' : 'Not selected'}
                      </span>
                    </div>
                    <div className="flex justify-between">
                      <span>Recipients:</span>
                      <span className="text-muted-foreground">
                        {recipientFields.length} recipient{recipientFields.length !== 1 ? 's' : ''}
                      </span>
                    </div>
                    <div className="flex justify-between">
                      <span>Currency:</span>
                      <span className="text-muted-foreground">{methods.watch('currency')}</span>
                    </div>
                    <div className="flex justify-between">
                      <span>Pricing:</span>
                      <span
                        className={`text-sm ${pricingValidation.isValid ? 'text-green-600' : 'text-yellow-600'}`}
                      >
                        {customerId && planVersionId
                          ? pricingValidation.isValid
                            ? 'Configured'
                            : 'Needs configuration'
                          : 'Plan or customer not selected'}
                      </span>
                    </div>
                  </div>
                </CardContent>
              </Card>
            </div>
          </div>
        </form>
      </Form>
    </div>
  )
}

const extractConfiguredComponents = (state: PriceComponentsState) => {
  return {
    parameterizedComponents: state.components.parameterized,

    overriddenComponents: state.components.overridden.map(c => ({
      componentId: c.componentId,
      component: mapOverrideComponentToSubscriptionComponent(c),
    })),

    extraComponents: state.components.extra.map(c => ({
      component: mapExtraComponentToSubscriptionComponent(c),
    })),

    removedComponentIds: state.components.removed,
  }
}

// Common fee mapping logic shared between parameterized and default components
const createSubscriptionFee = (
  priceComponent: PriceComponent,
  config?: {
    billingPeriod?: BillingPeriod
    committedCapacity?: bigint
    initialSlotCount?: number
  }
): SubscriptionFee => {
  const fee = new SubscriptionFee()

  if (priceComponent.fee?.feeType?.case === 'capacity') {
    const capacityFee = priceComponent.fee.feeType.value
    let selectedThreshold = capacityFee.thresholds?.[0] // Default to first threshold

    // If specific capacity is configured, find matching threshold
    if (config?.committedCapacity !== undefined) {
      const matchingThreshold = capacityFee.thresholds.find(
        t => t.includedAmount === config.committedCapacity
      )
      if (matchingThreshold) {
        selectedThreshold = matchingThreshold
      }
    }

    if (selectedThreshold) {
      fee.fee = {
        case: 'capacity',
        value: new SubscriptionFee_CapacitySubscriptionFee({
          rate: selectedThreshold.price,
          included: selectedThreshold.includedAmount,
          overageRate: selectedThreshold.perUnitOverage,
          metricId: capacityFee.metricId,
        }),
      }
    }
  } else if (priceComponent.fee?.feeType?.case === 'rate') {
    const rateFee = priceComponent.fee.feeType.value
    let selectedRate = rateFee.rates?.[0] // Default to first rate

    // If specific billing period is configured, find matching rate
    if (config?.billingPeriod !== undefined) {
      const matchingRate = rateFee.rates.find(r => {
        return r.term === config.billingPeriod
      })
      if (matchingRate) {
        selectedRate = matchingRate
      }
    }

    if (selectedRate) {
      fee.fee = {
        case: 'rate',
        value: new SubscriptionFee_RateSubscriptionFee({
          rate: selectedRate.price,
        }),
      }
    }
  } else if (priceComponent.fee?.feeType?.case === 'slot') {
    const slotFee = priceComponent.fee.feeType.value
    let unitRate = slotFee.rates?.[0]?.price || '0'

    // If specific billing period is configured and multiple rates exist, find matching rate
    if (config?.billingPeriod !== undefined && slotFee.rates.length > 1) {
      const matchingRate = slotFee.rates.find(r => {
        return r.term === config.billingPeriod
      })
      if (matchingRate) {
        unitRate = matchingRate.price
      }
    }

    const initialSlots = config?.initialSlotCount ?? slotFee.minimumCount ?? 1

    fee.fee = {
      case: 'slot',
      value: new SubscriptionFee_SlotSubscriptionFee({
        unit: slotFee.slotUnitName,
        unitRate,
        initialSlots,
        minSlots: slotFee.minimumCount,
        maxSlots: slotFee.quota,
      }),
    }
  } else if (priceComponent.fee?.feeType?.case === 'usage') {
    // Usage fees can be passed through directly as they share the same UsageFee type
    const usageFee = priceComponent.fee.feeType.value
    fee.fee = {
      case: 'usage',
      value: usageFee,
    }
  } else if (priceComponent.fee?.feeType?.case === 'oneTime') {
    const oneTimeFee = priceComponent.fee.feeType.value
    const quantity = oneTimeFee.quantity || 1
    const unitPrice = oneTimeFee.unitPrice || '0'
    const total = (parseFloat(unitPrice) * quantity).toString()

    fee.fee = {
      case: 'oneTime',
      value: new SubscriptionFee_OneTimeSubscriptionFee({
        rate: unitPrice,
        quantity,
        total,
      }),
    }
  } else if (priceComponent.fee?.feeType?.case === 'extraRecurring') {
    const recurringFee = priceComponent.fee.feeType.value
    const quantity = recurringFee.quantity || 1
    const unitPrice = recurringFee.unitPrice || '0'
    const total = (parseFloat(unitPrice) * quantity).toString()

    fee.fee = {
      case: 'recurring',
      value: new SubscriptionFee_ExtraRecurringSubscriptionFee({
        rate: unitPrice,
        quantity,
        total,
        billingType: recurringFee.billingType,
      }),
    }
  }

  return fee
}

const mapParameterizedComponentToSubscriptionComponent = (
  component: ComponentParameterization,
  priceComponent: PriceComponent
): SubscriptionComponentNewInternal => {
  const subscriptionComponent = new SubscriptionComponentNewInternal({
    priceComponentId: component.componentId,
    name: priceComponent.name,
    period:
      component.billingPeriod !== undefined
        ? component.billingPeriod === BillingPeriod.MONTHLY
          ? SubscriptionFeeBillingPeriod.MONTHLY
          : component.billingPeriod === BillingPeriod.QUARTERLY
            ? SubscriptionFeeBillingPeriod.QUARTERLY
            : component.billingPeriod === BillingPeriod.ANNUAL
              ? SubscriptionFeeBillingPeriod.YEARLY
              : SubscriptionFeeBillingPeriod.MONTHLY
        : SubscriptionFeeBillingPeriod.MONTHLY,
  })

  subscriptionComponent.fee = createSubscriptionFee(priceComponent, {
    billingPeriod: component.billingPeriod,
    committedCapacity: component.committedCapacity,
    initialSlotCount: component.initialSlotCount,
  })
  return subscriptionComponent
}

const mapDefaultComponentToSubscriptionComponent = (
  priceComponent: PriceComponent
): SubscriptionComponentNewInternal => {
  // Determine the billing period based on fee type
  let period = SubscriptionFeeBillingPeriod.MONTHLY // Default

  if (priceComponent.fee?.feeType?.case === 'usage') {
    // For usage fees, use the term from the usage fee
    const usageFee = priceComponent.fee.feeType.value
    period = mapBillingPeriodToSubscriptionPeriod(usageFee.term)
  } else if (priceComponent.fee?.feeType?.case === 'oneTime') {
    period = SubscriptionFeeBillingPeriod.ONE_TIME
  } else if (priceComponent.fee?.feeType?.case === 'rate') {
    // Use the first rate's term
    const rateFee = priceComponent.fee.feeType.value
    if (rateFee.rates?.[0]?.term !== undefined) {
      period = mapBillingPeriodToSubscriptionPeriod(rateFee.rates[0].term)
    }
  } else if (priceComponent.fee?.feeType?.case === 'slot') {
    // Use the first rate's term
    const slotFee = priceComponent.fee.feeType.value
    if (slotFee.rates?.[0]?.term !== undefined) {
      period = mapBillingPeriodToSubscriptionPeriod(slotFee.rates[0].term)
    }
  } else if (priceComponent.fee?.feeType?.case === 'capacity') {
    // Use the term from capacity fee
    const capacityFee = priceComponent.fee.feeType.value
    if (capacityFee.term !== undefined) {
      period = mapBillingPeriodToSubscriptionPeriod(capacityFee.term)
    }
  } else if (priceComponent.fee?.feeType?.case === 'extraRecurring') {
    // Use the term from extra recurring fee if available
    const recurringFee = priceComponent.fee.feeType.value
    if (recurringFee.term !== undefined) {
      period = mapBillingPeriodToSubscriptionPeriod(recurringFee.term)
    }
  }

  const subscriptionComponent = new SubscriptionComponentNewInternal({
    priceComponentId: priceComponent.id,
    name: priceComponent.name,
    period,
  })

  subscriptionComponent.fee = createSubscriptionFee(priceComponent) // No config = use defaults
  return subscriptionComponent
}

const mapBillingPeriodToSubscriptionPeriod = (
  billingPeriod: BillingPeriod
): SubscriptionFeeBillingPeriod => {
  switch (billingPeriod) {
    case BillingPeriod.MONTHLY:
      return SubscriptionFeeBillingPeriod.MONTHLY
    case BillingPeriod.QUARTERLY:
      return SubscriptionFeeBillingPeriod.QUARTERLY
    case BillingPeriod.ANNUAL:
      return SubscriptionFeeBillingPeriod.YEARLY
    default:
      return SubscriptionFeeBillingPeriod.MONTHLY
  }
}
