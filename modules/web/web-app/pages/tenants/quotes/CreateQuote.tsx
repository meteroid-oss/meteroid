import { PartialMessage } from '@bufbuild/protobuf'
import { disableQuery, useMutation } from '@connectrpc/connect-query'
import {
  Alert,
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
import {
  buildExistingProductRef,
  buildNewProductRef,
  buildPriceInputs,
  formDataToPrice,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing'
import { QuotePriceComponentsWrapper } from '@/features/quotes/QuotePriceComponentsWrapper'
import { QuoteView } from '@/features/quotes/QuoteView'
import { PriceComponentsState } from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import { useBasePath } from '@/hooks/useBasePath'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { mapDatev2 } from '@/lib/mapping'
import {
  getPrice,
  priceToSubscriptionFee,
  priceToSubscriptionPeriod,
} from '@/lib/mapping/priceToSubscriptionFee'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listCoupons } from '@/rpc/api/coupons/v1/coupons-CouponsService_connectquery'
import { ListCouponRequest_CouponFilter } from '@/rpc/api/coupons/v1/coupons_pb'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import {
  getInvoicingEntity,
  getInvoicingEntityProviders,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanWithVersionByVersionId,
  listPlans,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import {
  CreateQuoteCoupon,
  CreateQuoteCoupons,
  CreateQuote as CreateQuoteData,
  DetailedQuote,
  Quote,
  QuoteComponent,
} from '@/rpc/api/quotes/v1/models_pb'
import { createQuote } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { CreateQuoteRequest } from '@/rpc/api/quotes/v1/quotes_pb'
import {
  ActivationCondition,
  BankTransfer,
  CreateSubscriptionAddOn,
  CreateSubscriptionAddOns,
  CreateSubscriptionComponents,
  CreateSubscriptionComponents_ComponentOverride,
  CreateSubscriptionComponents_ExtraComponent,
  External,
  OnlinePayment,
  PaymentMethodsConfig,
} from '@/rpc/api/subscriptions/v1/models_pb'

const recipientSchema = z.object({
  name: z.string().min(1, 'Recipient name is required'),
  email: z.string().email('Valid email is required'),
})

const createQuoteSchema = z.object({
  quote_number: z.string().min(1, 'Quote number is required'),
  customer_id: z.string().min(1, 'Customer is required'),
  plan_version_id: z.string().min(1, 'Plan is required'),
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
  payment_methods_type: z.enum(['online', 'bankTransfer', 'external']).default('online'),
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

type PaymentMethodsType = 'online' | 'bankTransfer' | 'external'

const buildPaymentMethodsConfig = (type: PaymentMethodsType): PaymentMethodsConfig => {
  switch (type) {
    case 'online':
      return new PaymentMethodsConfig({ config: { case: 'online', value: new OnlinePayment() } })
    case 'bankTransfer':
      return new PaymentMethodsConfig({
        config: { case: 'bankTransfer', value: new BankTransfer() },
      })
    case 'external':
      return new PaymentMethodsConfig({ config: { case: 'external', value: new External() } })
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

  const methods = useZodForm({
    schema: createQuoteSchema,
    defaultValues: {
      quote_number: `Q-${new Date().toISOString().slice(0, 10).replace(/-/g, '')}-${nanoid(5)}`,
      customer_id: '',
      plan_version_id: '',
      net_terms: 30,
      recipients: [{ name: '', email: '' }],
      // Advanced settings defaults
      activation_condition: 'ON_START',
      payment_methods_type: 'online',
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

  // Fetch all plans for resolving trialing plan name
  const plansQuery = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  const planCurrency = planQuery.data?.plan?.version?.currency
  const planTrialConfig = planQuery.data?.plan?.version?.trialConfig
  const planType = planQuery.data?.plan?.plan?.planType
  const isFreePlan = planType === PlanType.FREE

  // Resolve trialing plan name
  const trialingPlanName = planTrialConfig?.trialingPlanId
    ? plansQuery.data?.plans.find(p => p.id === planTrialConfig.trialingPlanId)?.name
    : undefined

  const invoicingEntityQuery = useQuery(
    getInvoicingEntity,
    {
      id: customerQuery.data?.customer?.invoicingEntityId || '',
    },
    { enabled: Boolean(customerQuery.data?.customer?.invoicingEntityId) }
  )

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
  const [activationCondition, paymentMethodsType, chargeAutomatically] = methods.watch([
    'activation_condition',
    'payment_methods_type',
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

  // Auto-set payment methods type to online when OnCheckout is selected
  useEffect(() => {
    if (activationCondition === 'ON_CHECKOUT' && paymentMethodsType !== 'online') {
      methods.setValue('payment_methods_type', 'online')
    }
  }, [activationCondition, paymentMethodsType, methods])

  // Auto-disable chargeAutomatically when Bank or External payment methods selected
  useEffect(() => {
    if (
      (paymentMethodsType === 'bankTransfer' || paymentMethodsType === 'external') &&
      chargeAutomatically
    ) {
      methods.setValue('charge_automatically', false)
    }
  }, [paymentMethodsType, chargeAutomatically, methods])

  const onSubmit = async (data: CreateQuoteFormData) => {
    try {
      if (!planCurrency) throw new Error('Currency is required')

      const subscriptionComponents = new CreateSubscriptionComponents({
        parameterizedComponents: priceComponentsState.components.parameterized,
        overriddenComponents: priceComponentsState.components.overridden.map(c => {
          const pricingType = toPricingTypeFromFeeType(
            c.feeType,
            c.formData.usageModel as string | undefined
          )
          return new CreateSubscriptionComponents_ComponentOverride({
            componentId: c.componentId,
            name: c.name,
            price: wrapAsNewPriceEntries(
              buildPriceInputs(pricingType, c.formData as Record<string, unknown>, planCurrency)
            )[0],
          })
        }),
        extraComponents: priceComponentsState.components.extra.map(c => {
          const pricingType = toPricingTypeFromFeeType(
            c.feeType,
            c.formData.usageModel as string | undefined
          )
          return new CreateSubscriptionComponents_ExtraComponent({
            name: c.name,
            product: c.productId
              ? buildExistingProductRef(c.productId)
              : buildNewProductRef(c.name, c.feeType, c.formData as Record<string, unknown>),
            price: wrapAsNewPriceEntries(
              buildPriceInputs(pricingType, c.formData as Record<string, unknown>, planCurrency)
            )[0],
          })
        }),
        removeComponents: priceComponentsState.components.removed,
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
        currency: planCurrency,
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
        paymentMethodsConfig: buildPaymentMethodsConfig(data.payment_methods_type),
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
      currency: planCurrency,
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
    if (!planCurrency) return []

    const { parameterized, overridden, extra, removed } = priceComponentsState.components

    // Default plan components (not removed, not parameterized, not overridden)
    const defaultPlanComponents = priceComponentsData
      .filter(
        pc =>
          !removed.includes(pc.id) &&
          !parameterized.some(c => c.componentId === pc.id) &&
          !overridden.some(c => c.componentId === pc.id)
      )
      .flatMap(pc => {
        const price = getPrice(pc)
        if (!price) return []
        return [
          {
            id: pc.id,
            name: pc.name,
            period: priceToSubscriptionPeriod(price),
            fee: priceToSubscriptionFee(price),
          },
        ]
      })

    // Parameterized plan components
    const parameterizedComponents = parameterized.flatMap(c => {
      const pc = priceComponentsData.find(pc => pc.id === c.componentId)
      if (!pc) return []
      const price = getPrice(pc)
      if (!price) return []
      return [
        {
          id: pc.id,
          name: pc.name,
          period: priceToSubscriptionPeriod(price),
          fee: priceToSubscriptionFee(price, { initialSlotCount: c.initialSlotCount }),
        },
      ]
    })

    // Override components — derive display Price from formData
    const overriddenComponents = overridden.map(c => {
      const price = formDataToPrice(c.feeType, c.formData as Record<string, unknown>, planCurrency)
      return {
        id: c.componentId,
        name: c.name,
        period: priceToSubscriptionPeriod(price),
        fee: priceToSubscriptionFee(price),
      }
    })

    // Extra components — derive display Price from formData
    const extraComponents = extra.map(c => {
      const price = formDataToPrice(c.feeType, c.formData as Record<string, unknown>, planCurrency)
      return {
        id: c.name,
        name: c.name,
        period: priceToSubscriptionPeriod(price),
        fee: priceToSubscriptionFee(price),
      }
    })

    return [
      ...defaultPlanComponents,
      ...parameterizedComponents,
      ...overriddenComponents,
      ...extraComponents,
    ]
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

                    <div className="space-y-0 grid gap-2 md:grid md:grid-cols-12 items-center">
                      <Label className="col-span-4 text-xs text-muted-foreground">Currency</Label>
                      <span className="col-span-8 text-sm">
                        {planCurrency ?? 'Select a plan'}
                      </span>
                    </div>
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
              {planVersionId && customerId && planCurrency && (
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
                      currency={planCurrency}
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

                  {/* Trial Configuration */}
                  {planVersionId && (
                    <div className="border-t pt-4">
                      <h4 className="text-sm font-medium mb-4">Trial Configuration</h4>
                      {planTrialConfig?.durationDays ? (
                        <div className="space-y-4">
                          <Alert variant="default" className="text-sm">
                            <div className="space-y-1">
                              <p>
                                Plan default:{' '}
                                <span className="font-medium">{planTrialConfig.durationDays} days</span>
                                {trialingPlanName && (
                                  <>
                                    {' '}
                                    with <span className="font-medium">&quot;{trialingPlanName}&quot;</span>{' '}
                                    features
                                  </>
                                )}
                              </p>
                              <p className="text-muted-foreground text-xs">
                                {isFreePlan
                                  ? 'Free plan - no payment required.'
                                  : planTrialConfig.trialIsFree
                                    ? 'Free trial - payment method collected at checkout, billing starts after trial.'
                                    : 'Paid trial - billing starts immediately.'}
                              </p>
                            </div>
                          </Alert>
                          <InputFormField
                            name="trial_duration"
                            label="Trial Duration (days)"
                            type="number"
                            placeholder={String(planTrialConfig.durationDays)}
                            control={methods.control}
                            description="Leave empty to use plan default, or set to 0 to skip trial"
                            layout="vertical"
                          />
                        </div>
                      ) : (
                        <p className="text-sm text-muted-foreground">
                          No trial configured on this plan.
                        </p>
                      )}
                    </div>
                  )}

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
                        name="payment_methods_type"
                        label="Payment methods"
                        placeholder="Select payment method type"
                        control={methods.control}
                        labelTooltip={
                          <TooltipProvider delayDuration={100}>
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                              </TooltipTrigger>
                              <TooltipContent className="max-w-96">
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
                        <SelectItem
                          value="external"
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
                            name="charge_automatically"
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
                      <span className="text-muted-foreground">{planCurrency ?? '—'}</span>
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



