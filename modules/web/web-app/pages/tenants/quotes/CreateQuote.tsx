import { PartialMessage } from '@bufbuild/protobuf'
import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  DateFormField,
  Form,
  GenericFormField,
  InputFormField,
  TextareaFormField,
} from '@md/ui'
import { Eye, Plus, Save, Trash2 } from 'lucide-react'
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
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { getInvoicingEntity } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import {
  CreateQuote as CreateQuoteData,
  DetailedQuote,
  Quote,
  QuoteComponent,
} from '@/rpc/api/quotes/v1/models_pb'
import { createQuote } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { CreateQuoteRequest } from '@/rpc/api/quotes/v1/quotes_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import {
  ActivationCondition,
  CreateSubscriptionComponents,
  SubscriptionComponentNewInternal,
  SubscriptionFee,
  SubscriptionFee_CapacitySubscriptionFee,
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
  start_date: z.date(),
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
})

type CreateQuoteFormData = z.infer<typeof createQuoteSchema>

const nanoid = customAlphabet('ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789', 10)

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
      start_date: new Date(),
      net_terms: 30,
      recipients: [{ name: '', email: '' }],
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

  const onSubmit = async (data: CreateQuoteFormData) => {
    try {
      const configuredComponents = extractConfiguredComponents(priceComponentsState)

      const subscriptionComponents = new CreateSubscriptionComponents({
        parameterizedComponents: configuredComponents.parameterizedComponents,
        overriddenComponents: configuredComponents.overriddenComponents,
        extraComponents: configuredComponents.extraComponents,
        removeComponents: configuredComponents.removedComponentIds,
      })

      const createQuoteData = new CreateQuoteData({
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
        activationCondition: ActivationCondition.ON_START,
        attachments: [], // TODO: Add attachment support
        recipients: data.recipients.map(r => ({ name: r.name, email: r.email })),
        components: subscriptionComponents,
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

    const configuredComponents = extractConfiguredComponents(priceComponentsState)
    const mapParameterized = (c: ComponentParameterization) => {
      const pc = priceComponentsData.find(pc => pc.id === c.componentId)
      if (!pc) {
        return null
      }
      return mapParameterizedComponentToSubscriptionComponent(c, pc)
    }

    const allSubscriptionComponents = [
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
                  <CardDescription>
                    Basic information about this quote and subscription
                  </CardDescription>
                </CardHeader>
                <CardContent className="space-y-4">
                  <InputFormField
                    name="quote_number"
                    label="Quote Number"
                    control={methods.control}
                    placeholder="Q-12345"
                    description="Unique identifier for this quote"
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
                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
                    <div className="space-y-4">
                      <h4 className="text-sm font-medium">Subscription Dates</h4>
                      <DateFormField
                        name="start_date"
                        label="Start Date"
                        control={methods.control}
                        layout="vertical"
                      />
                      <DateFormField
                        name="end_date"
                        label="End Date (Optional)"
                        control={methods.control}
                        layout="vertical"
                      />
                      {/* <DateFormField
                        name="billing_start_date"
                        label="Billing Start Date (Optional)"
                        control={methods.control}
                        layout="vertical"
                        description="Leave empty to use start date"
                      /> */}
                    </div>

                    <div className="space-y-4">
                      <h4 className="text-sm font-medium">Configuration</h4>
                      {/* <InputFormField
                        name="trial_duration"
                        label="Trial Duration (days)"
                        control={methods.control}
                        type="number"
                        placeholder="0"
                        layout="vertical"
                      /> */}
                      <InputFormField
                        name="billing_day_anchor"
                        label="Billing Day"
                        control={methods.control}
                        type="number"
                        placeholder="Anniversary"
                        layout="vertical"
                      />
                      <DateFormField
                        name="expires_at"
                        label="Quote Expires On (Optional)"
                        control={methods.control}
                        layout="vertical"
                      />
                    </div>
                  </div>
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

              {/* Additional Information */}
              <Card>
                <CardHeader>
                  <CardTitle>Additional Information</CardTitle>
                </CardHeader>
                <CardContent className="space-y-4">
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
                          : 'Pending'}
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

  const fee = new SubscriptionFee()

  if (
    priceComponent.fee?.feeType?.case === 'capacity' &&
    component.committedCapacity !== undefined
  ) {
    const capacityFee = priceComponent.fee.feeType.value
    const selectedThreshold = capacityFee.thresholds.find(
      t => t.includedAmount === component.committedCapacity
    )
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
  } else if (
    priceComponent.fee?.feeType?.case === 'rate' &&
    component.billingPeriod !== undefined
  ) {
    const rateFee = priceComponent.fee.feeType.value
    const selectedRate = rateFee.rates.find(r => {
      return r.term === component.billingPeriod
    })
    if (selectedRate) {
      fee.fee = {
        case: 'rate',
        value: new SubscriptionFee_RateSubscriptionFee({
          rate: selectedRate.price,
        }),
      }
    }
  } else if (
    priceComponent.fee?.feeType?.case === 'slot' &&
    component.initialSlotCount !== undefined
  ) {
    const slotFee = priceComponent.fee.feeType.value
    let unitRate = '0'

    if (slotFee.rates.length > 1 && component.billingPeriod !== undefined) {
      const selectedRate = slotFee.rates.find(r => {
        return r.term === component.billingPeriod
      })
      unitRate = selectedRate?.price || slotFee.rates[0].price || '0'
    } else if (slotFee.rates.length > 0) {
      unitRate = slotFee.rates[0].price
    }

    fee.fee = {
      case: 'slot',
      value: new SubscriptionFee_SlotSubscriptionFee({
        unit: slotFee.slotUnitName,
        unitRate,
        initialSlots: component.initialSlotCount,
        minSlots: slotFee.minimumCount,
        maxSlots: slotFee.quota,
      }),
    }
  }

  subscriptionComponent.fee = fee
  return subscriptionComponent
}
