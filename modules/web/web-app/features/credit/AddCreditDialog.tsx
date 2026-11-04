import { timestampFromDate } from '@bufbuild/protobuf/wkt'
import { ConnectError } from '@connectrpc/connect'
import { skipToken, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  InputFormField,
  Label,
  RadioGroup,
  RadioGroupItem,
  Select,
  SelectContent,
  SelectFormField,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { InfoIcon } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { Control, useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { AdvancedSection, useHasFieldErrors } from '@/components/form'
import {
  ProductScopeField,
  applicableProductIds,
  defaultProductScope,
} from '@/features/credit/ProductScopeField'
import {
  DEFAULT_PRIORITY,
  Values,
  buildSchema,
  canSubmitCredit,
  monthsOrNever,
} from '@/features/credit/addCreditForm'
import { parseMinorUnits, parseMinorUnitsAtPrecision } from '@/features/credit/amounts'
import {
  addMonthsUtc,
  attemptPayload,
  nextCreditAttempt,
  settleCreditAttempt,
} from '@/features/credit/creditAttempt'
import { invalidateWallets } from '@/features/credit/labels'
import { paymentMethodLabel } from '@/features/credit/paymentMethods'
import {
  creditsToMoneyMinor,
  formatCredits,
  isValidUnitPrice,
  sameUnitPrice,
} from '@/features/credit/unitSale'
import { useLiveSubscriptions } from '@/features/credit/useLiveSubscriptions'
import { formatUsageFilter } from '@/features/subscriptions/UsageFilterFields'
import { useBasePath } from '@/hooks/useBasePath'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrency, formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { getCreditUnit } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'
import { getSubscriptionDetails } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { CreditAppliesTo, CreditGrantType } from '@/rpc/api/wallets/v1/models_pb'
import {
  grantCredit as grantCreditRpc,
  quoteCreditSale,
  sellCredit,
  topUpWallet,
} from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'
import { TopUpStatus } from '@/rpc/api/wallets/v1/wallets_pb'

import type { ScopeContext } from '@/features/credit/addCreditForm'
import type { CreditAttempt } from '@/features/credit/creditAttempt'
import type { CustomerPaymentMethod } from '@/rpc/api/customers/v1/models_pb'
import type { UsageFilter } from '@/rpc/api/subscriptions/v1/models_pb'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

type Props = {
  open: boolean
  onOpenChange: (open: boolean) => void
  wallet: Wallet
  customerCurrency: string
  paymentMethods: CustomerPaymentMethod[]
  defaultPaymentMethodId?: string
}

/** One entry point for adding credit: free credit, or paid credit charged now or invoiced. */
export const AddCreditDialog = ({
  open,
  onOpenChange,
  wallet,
  customerCurrency,
  paymentMethods,
  defaultPaymentMethodId,
}: Props) => {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const basePath = useBasePath()
  // One idempotency key per operation and request: a retried submit never adds credit twice.
  const attempts = useRef<Partial<Record<Operation, CreditAttempt>>>({})

  // Paid credit only funds invoice credit, sold in the customer's currency: money credit in
  // that same currency, credits of a unit at the unit's selling price in it.
  const creditUnit = wallet.creditUnit
  const canCharge = paymentMethods.length > 0
  const canInvoice =
    !!creditUnit || wallet.currency.toUpperCase() === customerCurrency.toUpperCase()
  // A money grant belongs only to a subscription billed in its currency.
  const subscriptions = useLiveSubscriptions(wallet.customerId, open).filter(
    subscription =>
      !!creditUnit || subscription.currency.toUpperCase() === wallet.currency.toUpperCase()
  )
  const canSellPaid = canCharge || canInvoice

  const unitQuery = useQuery(
    getCreditUnit,
    creditUnit && canSellPaid ? { id: creditUnit.id } : skipToken
  )
  const unit = unitQuery.data?.unit
  const listPrice = unit?.prices.find(
    price => price.currency.toUpperCase() === customerCurrency.toUpperCase()
  )?.unitPrice
  // The customer's agreed price applies only in the currency it was set in.
  const agreedPrice =
    wallet.unitPrice?.currency.toUpperCase() === customerCurrency.toUpperCase()
      ? wallet.unitPrice.unitPrice
      : undefined
  const defaultPrice = listPrice ? (agreedPrice ?? listPrice) : undefined
  const saleBlocker =
    creditUnit && unit
      ? !unit.productId
        ? `${creditUnit.pluralName} cannot be sold until a tax product is set in Settings, Credit units.`
        : !listPrice
          ? `${creditUnit.pluralName} have no selling price in ${customerCurrency}. Set one in Settings, Credit units.`
          : undefined
      : undefined

  const scope = useRef<ScopeContext | undefined>(undefined)
  const [schema] = useState(() => buildSchema(wallet.currency, creditUnit, () => scope.current))
  const methods = useZodForm({
    schema,
    defaultValues: {
      type: undefined,
      collection: canCharge ? 'charge' : 'invoice',
      amount: '',
      unitPrice: '',
      paymentMethodId: defaultPaymentMethodId ?? paymentMethods[0]?.id ?? '',
      expiresInMonths: '',
      priority: DEFAULT_PRIORITY,
      reason: '',
      reference: '',
      note: '',
      productScope: defaultProductScope,
      subscriptionId: null,
      appliesTo: 'ALL',
    },
  })
  const type = useWatch({ control: methods.control, name: 'type' })
  const collection = useWatch({ control: methods.control, name: 'collection' })
  const mode = type === 'paid' ? collection : 'grant'
  const amountInput = useWatch({ control: methods.control, name: 'amount' })
  const subscriptionId = useWatch({ control: methods.control, name: 'subscriptionId' })
  // Money credit for one subscription pays only what that subscription bills.
  const scopedSubscription = useQuery(
    getSubscriptionDetails,
    subscriptionId && !creditUnit && type === 'promotional' ? { subscriptionId } : skipToken
  )
  const scopedDetails = subscriptionId ? scopedSubscription.data : undefined
  // Add-ons do not expose their product here: with any, the products billed cannot be told.
  const subscriptionProductIds = useMemo(
    () =>
      scopedDetails && scopedDetails.addOns.length === 0
        ? new Set(
            scopedDetails.priceComponents
              .map(component => component.productId)
              .filter((id): id is string => !!id)
          )
        : undefined,
    [scopedDetails]
  )
  scope.current = subscriptionProductIds
    ? {
        subscriptionProductIds,
        planName: scopedDetails?.subscription?.planName ?? 'This subscription',
      }
    : undefined
  const unitPriceInput = useWatch({ control: methods.control, name: 'unitPrice' })
  const [appliesTo, productScopeMode, priority, reference] = useWatch({
    control: methods.control,
    name: ['appliesTo', 'productScope.mode', 'priority', 'reference'],
  })
  const hasAdvancedGrantValues =
    subscriptionId !== null ||
    appliesTo !== 'ALL' ||
    productScopeMode !== 'all' ||
    priority !== DEFAULT_PRIORITY ||
    !!reference.trim()
  const advancedGrantError = useHasFieldErrors(methods.control, ADVANCED_GRANT_FIELDS)
  const unitPriceError = useHasFieldErrors(methods.control, ['unitPrice'])
  // The price is prefilled: changing it is an override for this purchase only.
  const priceOverridden = !!defaultPrice && !sameUnitPrice(unitPriceInput, defaultPrice)

  const { getFieldState, setValue, resetField } = methods
  useEffect(() => {
    if (defaultPrice && !getFieldState('unitPrice').isDirty) setValue('unitPrice', defaultPrice)
  }, [defaultPrice, getFieldState, setValue])

  useEffect(() => {
    if (saleBlocker && type === 'paid') resetField('type')
  }, [saleBlocker, type, resetField])

  // A price is only sent as this purchase's own when the operator changed the default one.
  const sale = useMemo(() => {
    if (!creditUnit || type !== 'paid' || saleBlocker) return null
    const credits = parseMinorUnitsAtPrecision(amountInput, creditUnit.precision)
    if (credits === null || credits <= 0n || !isValidUnitPrice(unitPriceInput)) return null
    const unitPrice =
      defaultPrice !== undefined && sameUnitPrice(unitPriceInput, defaultPrice)
        ? undefined
        : unitPriceInput.trim()
    return { amount: credits, unitPrice, appliedPrice: unitPriceInput.trim() }
  }, [creditUnit, type, saleBlocker, amountInput, unitPriceInput, defaultPrice])
  const debouncedSale = useDebounceValue(sale, 400)
  const quoteQuery = useQuery(
    quoteCreditSale,
    debouncedSale
      ? {
          customerId: wallet.customerId,
          walletId: wallet.id,
          amount: debouncedSale.amount,
          unitPrice: debouncedSale.unitPrice,
        }
      : skipToken,
    { retry: false }
  )
  const settled = sale !== null && debouncedSale === sale
  const quote = settled ? quoteQuery.data : undefined
  const quoteError = settled ? quoteQuery.error : null

  const onSuccess = () => invalidateWallets(queryClient)
  const grantCredit = useMutation(grantCreditRpc, { onSuccess })
  const sell = useMutation(sellCredit, { onSuccess })
  const topUp = useMutation(topUpWallet, { onSuccess })
  const pending = grantCredit.isPending || sell.isPending || topUp.isPending

  const onSubmit = async (values: Values) => {
    const amount =
      (creditUnit
        ? parseMinorUnitsAtPrecision(values.amount, creditUnit.precision)
        : parseMinorUnits(values.amount, wallet.currency)) ?? 0n
    const unitPrice =
      creditUnit &&
      mode !== 'grant' &&
      !(defaultPrice && sameUnitPrice(values.unitPrice, defaultPrice))
        ? values.unitPrice.trim()
        : undefined
    // The price shown: a sale or top-up at any other is refused, never charged.
    const expectedUnitPrice = creditUnit && mode !== 'grant' ? values.unitPrice.trim() : undefined
    const months = monthsOrNever(values.expiresInMonths)
    const grantFields =
      values.type === 'outside'
        ? {
            grantType: CreditGrantType.PAID,
            priority: DEFAULT_PRIORITY,
            months: undefined,
            applicableProductIds: [],
            appliesTo: CreditAppliesTo.ALL,
          }
        : {
            grantType: CreditGrantType.PROMOTIONAL,
            priority: values.priority,
            months,
            applicableProductIds: creditUnit ? [] : applicableProductIds(values.productScope),
            subscriptionId: values.subscriptionId ?? undefined,
            appliesTo:
              !creditUnit && values.appliesTo === 'USAGE'
                ? CreditAppliesTo.USAGE
                : CreditAppliesTo.ALL,
          }
    const request =
      mode === 'grant'
        ? {
            amount,
            reason: values.reason,
            reference: values.reference || undefined,
            ...grantFields,
          }
        : mode === 'invoice'
          ? { amount, months, note: values.note || undefined, unitPrice, expectedUnitPrice }
          : {
              amount,
              months,
              paymentMethodId: values.paymentMethodId,
              unitPrice,
              expectedUnitPrice,
            }
    const next = nextCreditAttempt(
      attempts.current[mode],
      attemptPayload(request),
      new Date(),
      () => crypto.randomUUID()
    )
    if ('blocked' in next) {
      toast.error(
        'The last attempt may have gone through. Close this dialog and check the credit before trying other values.'
      )
      return
    }
    const attempt = next.attempt
    attempts.current[mode] = attempt
    // Resolved once per key: a retry sends the same expiry, which the key's request includes.
    const expiresAt = (term?: number) =>
      term ? timestampFromDate(addMonthsUtc(attempt.frozenAt, term)) : undefined
    try {
      if (mode === 'grant') {
        const { months: grantMonths, ...grant } = grantFields
        await grantCredit.mutateAsync({
          customerId: wallet.customerId,
          walletId: wallet.id,
          amount,
          expiresAt: expiresAt(grantMonths),
          reason: values.reason,
          reference: values.reference || undefined,
          idempotencyKey: attempt.key,
          ...grant,
        })
        toast.success(creditUnit ? `${formatCredits(amount, creditUnit)} added` : 'Credit added')
      } else if (mode === 'invoice') {
        // Counted from the payment: the term stays relative, and the key's request with it.
        const response = await sell.mutateAsync({
          customerId: wallet.customerId,
          walletId: wallet.id,
          amount,
          expiresInMonths: months,
          note: values.note || undefined,
          idempotencyKey: attempt.key,
          unitPrice,
          expectedUnitPrice,
        })
        toast.success('Invoice issued; the credit is added once it is paid')
        onOpenChange(false)
        navigate(`${basePath}/invoices/${response.invoiceId}`)
        return
      } else {
        const response = await topUp.mutateAsync({
          customerId: wallet.customerId,
          walletId: wallet.id,
          amount,
          paymentMethodId: values.paymentMethodId,
          idempotencyKey: attempt.key,
          expiresAt: expiresAt(months),
          unitPrice,
          expectedUnitPrice,
        })
        const charged = formatCurrency(response.chargedAmount, response.currency)
        switch (response.status) {
          case TopUpStatus.SUCCEEDED:
            toast.success(`${charged} charged; the credit is in the wallet`)
            break
          case TopUpStatus.REQUIRES_ACTION:
            toast.info(
              `The bank asks the customer to authenticate the ${charged} charge; the credit is added once they do`
            )
            break
          default:
            toast.info(
              `${charged} charge submitted; the credit is added once the payment completes`
            )
        }
      }
      onOpenChange(false)
    } catch (error) {
      attempts.current[mode] = settleCreditAttempt(attempt, error)
      toast.error(ConnectError.from(error).rawMessage || 'The credit could not be added')
    }
  }

  const unitPriceField = (
    <InputFormField
      control={methods.control}
      name="unitPrice"
      label={`Price per credit (${customerCurrency})`}
      inputMode="decimal"
      description={
        listPrice && defaultPrice
          ? sameUnitPrice(unitPriceInput, defaultPrice)
            ? agreedPrice
              ? `The customer's agreed price (list price ${formatCurrencyNoRounding(listPrice, customerCurrency)}).`
              : 'The list price.'
            : `For this purchase only; the ${agreedPrice ? "customer's agreed" : 'list'} price is ${formatCurrencyNoRounding(defaultPrice, customerCurrency)}.`
          : undefined
      }
    />
  )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Add credit</DialogTitle>
          <DialogDescription>
            {!creditUnit
              ? `Credit is applied to the customer's finalized invoices in ${wallet.currency}.`
              : `Credits pay for usage priced in ${creditUnit.pluralName}.`}
          </DialogDescription>
        </DialogHeader>
        <Form {...methods}>
          <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-4">
            <ChoiceField
              name="type"
              label="Type"
              control={methods.control}
              options={[
                creditUnit
                  ? {
                      value: 'promotional',
                      label: 'Free credit',
                      hint: 'Nothing is charged',
                    }
                  : {
                      value: 'promotional',
                      label: 'Free credit',
                      hint: 'Goodwill or promotion: applied before tax, like a discount',
                    },
                ...(!creditUnit
                  ? [
                      {
                        value: 'outside',
                        label: 'Account balance',
                        hint: 'Already paid to you outside Meteroid: applied after tax',
                      },
                    ]
                  : []),
                ...(canSellPaid
                  ? [
                      creditUnit
                        ? {
                            value: 'paid',
                            label: `Sell ${creditUnit.pluralName}`,
                            hint: defaultPrice
                              ? `${formatCurrencyNoRounding(defaultPrice, customerCurrency)} per credit + tax`
                              : 'The customer pays for them',
                            disabled: !!saleBlocker || unitQuery.isLoading,
                          }
                        : {
                            value: 'paid',
                            label: 'Sell credit',
                            hint: 'Charge a payment method or send an invoice',
                          },
                    ]
                  : []),
              ]}
            />
            {saleBlocker && <p className="text-xs text-muted-foreground">{saleBlocker}</p>}
            {type === 'paid' && (
              <ChoiceField
                name="collection"
                label="Payment"
                control={methods.control}
                options={[
                  ...(canCharge
                    ? [
                        {
                          value: 'charge',
                          label: 'Charge now',
                          hint: 'Added once the payment succeeds',
                        },
                      ]
                    : []),
                  ...(canInvoice
                    ? [
                        {
                          value: 'invoice',
                          label: 'Send an invoice',
                          hint: 'Added once the invoice is paid',
                        },
                      ]
                    : []),
                ]}
              />
            )}
            <InputFormField
              control={methods.control}
              name="amount"
              label={
                creditUnit ? `Quantity (${creditUnit.pluralName})` : `Amount (${wallet.currency})`
              }
              inputMode="decimal"
              autoFocus
            />
            {creditUnit && type === 'paid' && (
              <>
                {!defaultPrice && unitPriceField}
                {sale && (
                  <SaleSummary
                    estimate={creditsToMoneyMinor(
                      sale.amount,
                      creditUnit.precision,
                      sale.appliedPrice,
                      customerCurrency
                    )}
                    currency={customerCurrency}
                    quote={quote}
                    error={quoteError?.rawMessage}
                  />
                )}
              </>
            )}
            {mode === 'charge' && (
              <SelectFormField
                control={methods.control}
                name="paymentMethodId"
                label="Payment method"
              >
                {paymentMethods.map(method => (
                  <SelectItem key={method.id} value={method.id}>
                    {paymentMethodLabel(method)}
                    {method.id === defaultPaymentMethodId ? ' (default)' : ''}
                  </SelectItem>
                ))}
              </SelectFormField>
            )}
            {type === 'outside' && (
              <>
                <InputFormField
                  control={methods.control}
                  name="reason"
                  label="Reason"
                  required
                  placeholder="e.g. Bank transfer received on 12 March"
                  maxLength={1024}
                />
                <InputFormField
                  control={methods.control}
                  name="reference"
                  label="Payment reference"
                  placeholder="Invoice or receipt number"
                  description="Required for French entities."
                  maxLength={255}
                />
              </>
            )}
            {type === 'promotional' && (
              <>
                <InputFormField
                  control={methods.control}
                  name="expiresInMonths"
                  label="Expires after (months, optional)"
                  inputMode="numeric"
                  description="Leave empty for no expiry."
                />
                <InputFormField
                  control={methods.control}
                  name="reason"
                  label="Reason"
                  required
                  placeholder="e.g. Goodwill for the March outage"
                  maxLength={1024}
                />
                <AdvancedSection
                  defaultOpen={hasAdvancedGrantValues}
                  forceOpen={advancedGrantError}
                >
                  {subscriptions.length > 0 && (
                    <SubscriptionField control={methods.control} subscriptions={subscriptions} />
                  )}
                  {/* Credit in a credit unit pays the usage priced in it, for every product. */}
                  {!creditUnit && (
                    <>
                      <ChoiceField
                        name="appliesTo"
                        label="Pays"
                        control={methods.control}
                        options={[
                          { value: 'ALL', label: 'All charges', hint: 'Fees and usage alike' },
                          { value: 'USAGE', label: 'Usage only', hint: 'Not subscription fees' },
                        ]}
                      />
                      <ProductScopeField
                        control={methods.control}
                        allowedProductIds={subscriptionProductIds}
                        allowedHint={
                          subscriptionProductIds
                            ? `Only the products ${scopedDetails?.subscription?.planName ?? 'this subscription'} bills.`
                            : undefined
                        }
                      />
                    </>
                  )}
                  <InputFormField
                    control={methods.control}
                    name="priority"
                    label="Priority"
                    type="number"
                    description="Lower numbers are used first."
                    labelTooltip={
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <InfoIcon className="h-3.5 w-3.5 cursor-help text-muted-foreground" />
                        </TooltipTrigger>
                        <TooltipContent className="max-w-80">
                          Narrowest credit goes first: a subscription&apos;s own, then selected
                          products, then usage-only. Then lower priority, sooner expiry, free before
                          paid.
                        </TooltipContent>
                      </Tooltip>
                    }
                  />
                  <InputFormField
                    control={methods.control}
                    name="reference"
                    label="Reference (optional)"
                    placeholder="Contract or ticket number"
                    maxLength={255}
                  />
                </AdvancedSection>
              </>
            )}
            {mode === 'charge' && (
              <InputFormField
                control={methods.control}
                name="expiresInMonths"
                label="Expires after (months, optional)"
                inputMode="numeric"
                description="Leave empty for no expiry."
              />
            )}
            {mode === 'invoice' && (
              <>
                <InputFormField
                  control={methods.control}
                  name="expiresInMonths"
                  label="Expires after (months, optional)"
                  inputMode="numeric"
                  description="Counted from the payment. Leave empty for no expiry."
                />
                <InputFormField
                  control={methods.control}
                  name="note"
                  label="Note on the invoice (optional)"
                  maxLength={1024}
                />
              </>
            )}
            {creditUnit && type === 'paid' && defaultPrice && (
              <AdvancedSection defaultOpen={priceOverridden} forceOpen={unitPriceError}>
                {unitPriceField}
              </AdvancedSection>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button
                type="submit"
                disabled={
                  !canSubmitCredit(type, pending) || (!!creditUnit && mode !== 'grant' && !quote)
                }
              >
                {pending
                  ? 'Working…'
                  : creditUnit && mode === 'charge' && quote
                    ? `Charge ${formatCurrency(quote.total, quote.currency)}`
                    : SUBMIT_LABEL[mode]}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}

const SUBMIT_LABEL: Record<'grant' | 'charge' | 'invoice', string> = {
  grant: 'Add credit',
  charge: 'Charge and add credit',
  invoice: 'Issue invoice',
}

type ChoiceOption = { value: string; label: string; hint: string; disabled?: boolean }

type Operation = 'grant' | 'charge' | 'invoice'

const ANY_SUBSCRIPTION = '__any__'

const ADVANCED_GRANT_FIELDS = [
  'subscriptionId',
  'appliesTo',
  'productScope',
  'priority',
  'reference',
] as const

const SubscriptionField = ({
  control,
  subscriptions,
}: {
  control: Control<Values>
  subscriptions: { id: string; planName: string; startDate: string; usageFilter?: UsageFilter }[]
}) => (
  <FormField
    control={control}
    name="subscriptionId"
    render={({ field }) => (
      <FormItem className="space-y-2">
        <FormLabel>Can be used for</FormLabel>
        <Select
          value={field.value ?? ANY_SUBSCRIPTION}
          onValueChange={value => field.onChange(value === ANY_SUBSCRIPTION ? null : value)}
        >
          <FormControl>
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
          </FormControl>
          <SelectContent>
            <SelectItem value={ANY_SUBSCRIPTION}>
              All of this customer&apos;s subscriptions
            </SelectItem>
            {subscriptions.map(subscription => (
              <SelectItem key={subscription.id} value={subscription.id}>
                {subscription.planName}
                {subscription.usageFilter
                  ? ` (${formatUsageFilter(subscription.usageFilter)})`
                  : ''}{' '}
                only
                {subscriptions.filter(other => other.planName === subscription.planName).length > 1
                  ? ` · from ${subscription.startDate}`
                  : ''}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground">Credit for one subscription ends with it.</p>
        <FormMessage />
      </FormItem>
    )}
  />
)

/** The sale's money: the server's quote once in, the pre-tax estimate meanwhile. */
const SaleSummary = ({
  estimate,
  currency,
  quote,
  error,
}: {
  estimate: bigint | null
  currency: string
  quote?: { net: bigint; tax: bigint; total: bigint; currency: string }
  error?: string
}) => {
  if (error) {
    return (
      <p role="alert" className="text-sm text-destructive">
        {error}
      </p>
    )
  }
  if (quote) {
    const money = (amount: bigint) => formatCurrency(amount, quote.currency)
    return (
      <div className="rounded-md border border-border bg-muted px-3 py-2 text-sm">
        <span className="font-medium">Total {money(quote.total)}</span>{' '}
        <span className="text-muted-foreground">
          {quote.tax > 0n
            ? `(net ${money(quote.net)} + ${money(quote.tax)} tax)`
            : '(no tax applies)'}
        </span>
      </div>
    )
  }
  return (
    <div className="rounded-md border border-border bg-muted px-3 py-2 text-sm text-muted-foreground">
      {estimate !== null ? `${formatCurrency(estimate, currency)} + tax` : 'Calculating…'}
    </div>
  )
}

const ChoiceField = ({
  name,
  label,
  control,
  options,
}: {
  name: 'type' | 'collection' | 'appliesTo'
  label: string
  control: Control<Values>
  options: ChoiceOption[]
}) => (
  <FormField
    control={control}
    name={name}
    render={({ field }) => (
      <FormItem className="space-y-2">
        <FormLabel>{label}</FormLabel>
        <FormControl>
          <RadioGroup
            value={field.value}
            onValueChange={field.onChange}
            className="grid gap-2 sm:grid-cols-2"
          >
            {options.map(option => {
              const id = `add-credit-${name}-${option.value}`
              return (
                <Label
                  key={option.value}
                  htmlFor={id}
                  className="flex cursor-pointer items-start gap-2 rounded-md border border-border p-3 font-normal has-[[data-state=checked]]:border-primary has-[:disabled]:cursor-not-allowed has-[:disabled]:opacity-60"
                >
                  <RadioGroupItem
                    value={option.value}
                    id={id}
                    className="mt-0.5"
                    disabled={option.disabled}
                  />
                  <span className="space-y-0.5">
                    <span className="block text-sm font-medium">{option.label}</span>
                    <span className="block text-xs text-muted-foreground">{option.hint}</span>
                  </span>
                </Label>
              )
            })}
          </RadioGroup>
        </FormControl>
        <FormMessage />
      </FormItem>
    )}
  />
)
