import { disableQuery } from '@connectrpc/connect-query'
import {
  Alert,
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
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanWithVersionByVersionId,
  listPlans,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { ListPlansRequest_SortBy } from '@/rpc/api/plans/v1/plans_pb'
import { ActivationCondition } from '@/rpc/api/subscriptions/v1/models_pb'

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

export const StepSettings = () => {
  const { previousStep, nextStep } = useWizard()
  const [state, setState] = useAtom(createSubscriptionAtom)

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

  // Fetch all plans for resolving trialing plan name
  const plansQuery = useQuery(listPlans, {
    sortBy: ListPlansRequest_SortBy.NAME_ASC,
  })

  const planTrialConfig = planQuery.data?.plan?.version?.trialConfig
  const planType = planQuery.data?.plan?.plan?.planType
  const isFreePlan = planType === PlanType.FREE

  // Resolve trialing plan name
  const trialingPlanName = planTrialConfig?.trialingPlanId
    ? plansQuery.data?.plans.find(p => p.id === planTrialConfig.trialingPlanId)?.name
    : undefined

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
      paymentMethodsType: state.paymentMethodsType,
      netTerms: state.netTerms,
      invoiceMemo: state.invoiceMemo,
      invoiceThreshold: state.invoiceThreshold,
      purchaseOrder: state.purchaseOrder,
      autoAdvanceInvoices: state.autoAdvanceInvoices,
      chargeAutomatically: state.chargeAutomatically,
      skipPastInvoices: state.skipPastInvoices,
    },
  })

  // Watch activation condition, payment methods type, charge automatically, and fromDate for cross-validation
  const [activationCondition, paymentMethodsType, chargeAutomatically, fromDate] = methods.watch([
    'activationCondition',
    'paymentMethodsType',
    'chargeAutomatically',
    'fromDate',
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

  // Auto-set payment methods type to Online when OnCheckout is selected
  useEffect(() => {
    if (activationCondition === 'ON_CHECKOUT' && paymentMethodsType !== 'online') {
      methods.setValue('paymentMethodsType', 'online')
    }
  }, [activationCondition, paymentMethodsType, methods])

  // Auto-disable chargeAutomatically when Bank or External payment methods selected
  useEffect(() => {
    if (
      (paymentMethodsType === 'bankTransfer' || paymentMethodsType === 'external') &&
      chargeAutomatically
    ) {
      methods.setValue('chargeAutomatically', false)
    }
  }, [paymentMethodsType, chargeAutomatically, methods])

  // Auto-disable skipPastInvoices when activation condition is not ON_START
  useEffect(() => {
    if (activationCondition !== 'ON_START' && methods.getValues('skipPastInvoices')) {
      methods.setValue('skipPastInvoices', false)
    }
  }, [activationCondition, methods])

  // Reset skipPastInvoices when fromDate changes to future or is undefined
  useEffect(() => {
    if (!fromDate || fromDate >= new Date()) {
      methods.setValue('skipPastInvoices', false)
    }
  }, [fromDate, methods])

  const onSubmit = async (data: z.infer<typeof schema>) => {
    setState({
      ...state,
      startDate: data.fromDate,
      endDate: data.toDate,
      billingDay: data.billingDay,
      trialDuration: data.trialDuration,
      activationCondition: activationConditionFromString(data.activationCondition),
      paymentMethodsType: data.paymentMethodsType,
      netTerms: data.netTerms,
      invoiceMemo: data.invoiceMemo,
      invoiceThreshold: data.invoiceThreshold,
      purchaseOrder: data.purchaseOrder,
      autoAdvanceInvoices: data.autoAdvanceInvoices,
      chargeAutomatically: data.chargeAutomatically,
      skipPastInvoices: data.skipPastInvoices,
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
                {/* Show only when start date is in the past */}
                {fromDate && fromDate < new Date(new Date().setHours(0, 0, 0, 0)) && (
                  <div className="pt-4 border-t space-y-2">
                    <div className="flex items-center gap-1">
                      <Label
                        className={`text-sm font-medium ${activationCondition !== 'ON_START' ? 'text-muted-foreground' : ''}`}
                      >
                        Migration Settings
                      </Label>
                      <TooltipProvider delayDuration={100}>
                        <Tooltip>
                          <TooltipTrigger asChild>
                            <InfoIcon className="h-4 w-4 text-muted-foreground cursor-help" />
                          </TooltipTrigger>
                          <TooltipContent className="max-w-xs">
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
                <CardTitle className="text-base">Trial Configuration</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                {planTrialConfig?.durationDays ? (
                  <>
                    <Alert variant="default" className="text-sm">
                      <div className="space-y-1">
                        <p>
                          Plan default:{' '}
                          <span className="font-medium">{planTrialConfig.durationDays} days</span>
                          {trialingPlanName && (
                            <>
                              {' '}
                              with{' '}
                              <span className="font-medium">
                                &quot;{trialingPlanName}&quot;
                              </span>{' '}
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
                      name="trialDuration"
                      label="Trial Duration (days)"
                      type="number"
                      placeholder={String(planTrialConfig.durationDays)}
                      control={methods.control}
                      description="Leave empty to use plan default, or set to 0 to skip trial"
                    />
                  </>
                ) : (
                  <p className="text-sm text-muted-foreground">No trial configured on this plan.</p>
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
                    name="paymentMethodsType"
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
    paymentMethodsType: z.enum(['online', 'bankTransfer', 'external']).default('online'),
    netTerms: z.number().min(0),
    invoiceMemo: z.string().optional(),
    invoiceThreshold: z.string().optional(),
    purchaseOrder: z.string().optional(),
    autoAdvanceInvoices: z.boolean().default(true),
    chargeAutomatically: z.boolean().default(true),
    skipPastInvoices: z.boolean().default(false),
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
