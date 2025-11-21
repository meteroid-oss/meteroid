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

import { PageSection } from '@/components/layouts/shared/PageSection'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { getInvoicingEntityProviders } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { ActivationCondition, PaymentStrategy } from '@/rpc/api/subscriptions/v1/models_pb'
import { disableQuery } from '@connectrpc/connect-query'

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

const paymentStrategyToString = (strategy: PaymentStrategy): 'AUTO' | 'BANK' | 'EXTERNAL' => {
  switch (strategy) {
    case PaymentStrategy.AUTO:
      return 'AUTO'
    case PaymentStrategy.BANK:
      return 'BANK'
    case PaymentStrategy.EXTERNAL:
      return 'EXTERNAL'
    default:
      return 'AUTO'
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

export const StepSettings = () => {
  const { previousStep, nextStep } = useWizard()
  const [state, setState] = useAtom(createSubscriptionAtom)

  // Fetch customer to get invoicing entity ID
  const customerQuery = useQuery(
    getCustomerById,
    { id: state.customerId! },
    { enabled: !!state.customerId }
  )

  const invoicingEntityId = customerQuery.data?.customer?.invoicingEntityId

  // Fetch invoicing entity providers to check if online payment is available
  const providersQuery = useQuery(
    getInvoicingEntityProviders,
    invoicingEntityId ? { id: invoicingEntityId } : disableQuery
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
      trialDuration: state.trialDuration,
      activationCondition:
        state.activationCondition !== undefined
          ? activationConditionToString(state.activationCondition)
          : 'ON_START',
      paymentStrategy:
        state.paymentStrategy !== undefined
          ? paymentStrategyToString(state.paymentStrategy)
          : 'AUTO',
      netTerms: state.netTerms,
      invoiceMemo: state.invoiceMemo,
      invoiceThreshold: state.invoiceThreshold,
      purchaseOrder: state.purchaseOrder,
      autoAdvanceInvoices: state.autoAdvanceInvoices,
      chargeAutomatically: state.chargeAutomatically,
    },
  })

  // Watch activation condition, payment strategy, and charge automatically for cross-validation
  const [activationCondition, paymentStrategy, chargeAutomatically] = methods.watch([
    'activationCondition',
    'paymentStrategy',
    'chargeAutomatically',
  ])

  // Auto-disable chargeAutomatically and reset activationCondition when no online provider
  useEffect(() => {
    if (!isLoadingProviders && !hasOnlinePaymentProvider) {
      const currentActivation = methods.getValues('activationCondition')
      const currentCharge = methods.getValues('chargeAutomatically')

      if (currentActivation === 'ON_CHECKOUT') {
        methods.setValue('activationCondition', 'ON_START')
      }
      if (currentCharge) {
        methods.setValue('chargeAutomatically', false)
      }
    }
  }, [hasOnlinePaymentProvider, isLoadingProviders, methods])

  // Auto-set payment strategy to Auto when OnCheckout is selected
  useEffect(() => {
    if (activationCondition === 'ON_CHECKOUT' && paymentStrategy !== 'AUTO') {
      methods.setValue('paymentStrategy', 'AUTO')
    }
  }, [activationCondition, paymentStrategy, methods])

  // Auto-disable chargeAutomatically when Bank or External payment strategy is selected
  useEffect(() => {
    if ((paymentStrategy === 'BANK' || paymentStrategy === 'EXTERNAL') && chargeAutomatically) {
      methods.setValue('chargeAutomatically', false)
    }
  }, [paymentStrategy, chargeAutomatically, methods])

  const onSubmit = async (data: z.infer<typeof schema>) => {
    setState({
      ...state,
      startDate: data.fromDate,
      endDate: data.toDate,
      billingDay: data.billingDay,
      trialDuration: data.trialDuration,
      activationCondition: activationConditionFromString(data.activationCondition),
      paymentStrategy: paymentStrategyFromString(data.paymentStrategy),
      netTerms: data.netTerms,
      invoiceMemo: data.invoiceMemo,
      invoiceThreshold: data.invoiceThreshold,
      purchaseOrder: data.purchaseOrder,
      autoAdvanceInvoices: data.autoAdvanceInvoices,
      chargeAutomatically: data.chargeAutomatically,
    })
    nextStep()
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-6">
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
                <InputFormField
                  name="trialDuration"
                  label="Trial Duration (days)"
                  type="number"
                  containerClassName="hidden"
                  placeholder="7"
                  control={methods.control}
                />
                {/* TODO */}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-base">Billing Configuration</CardTitle>
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
                  placeholder="30"
                  control={methods.control}
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
                    name="paymentStrategy"
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
                            Bank Transfer and External options restrict payment methods accordingly.
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
                    <SelectItem value="EXTERNAL" disabled={activationCondition === 'ON_CHECKOUT'}>
                      External
                      {activationCondition === 'ON_CHECKOUT' && ' (unavailable with Checkout)'}
                    </SelectItem>
                  </SelectFormField>
                </div>
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
                      name="chargeAutomatically"
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
                            {!isLoadingProviders && !hasOnlinePaymentProvider
                              ? ' (requires a payment provider)'
                              : (paymentStrategy === 'BANK' || paymentStrategy === 'EXTERNAL') &&
                                  field.value
                                ? ' (disabled by payment strategy)'
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
              </CardContent>
            </Card>
          </div>
        </PageSection>

        <div className="flex gap-2 justify-end">
          <Button onClick={previousStep} variant="secondary">
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
    trialDuration: z.number().min(0).optional(),
    activationCondition: z.enum(['ON_START', 'ON_CHECKOUT', 'MANUAL']),
    paymentStrategy: z.enum(['AUTO', 'BANK', 'EXTERNAL']).default('AUTO'),
    netTerms: z.number().min(0),
    invoiceMemo: z.string().optional(),
    invoiceThreshold: z.string().optional(),
    purchaseOrder: z.string().optional(),
    autoAdvanceInvoices: z.boolean().default(true),
    chargeAutomatically: z.boolean().default(true),
  })
  .refine(data => !data.toDate || data.toDate > data.fromDate, {
    message: 'Must be greater than the start date',
    path: ['toDate'],
  })
  .refine(
    data => {
      // OnCheckout requires Auto payment strategy
      if (data.activationCondition === 'ON_CHECKOUT' && data.paymentStrategy !== 'AUTO') {
        return false
      }
      return true
    },
    {
      message: 'OnCheckout activation requires Auto payment strategy',
      path: ['activationCondition'],
    }
  )
  .refine(
    data => {
      // ChargeAutomatically doesn't make sense with Bank or External strategies
      if (
        data.chargeAutomatically &&
        (data.paymentStrategy === 'BANK' || data.paymentStrategy === 'EXTERNAL')
      ) {
        return false
      }
      return true
    },
    {
      message: 'Automatic charging requires Auto payment strategy',
      path: ['chargeAutomatically'],
    }
  )
