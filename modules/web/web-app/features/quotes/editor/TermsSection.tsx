import {
  DatePicker,
  GenericFormField,
  InputFormField,
  Label,
  Select,
  SelectContent,
  SelectFormField,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
  TextareaFormField,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { addYears } from 'date-fns'
import { InfoIcon } from 'lucide-react'
import { useState } from 'react'
import { useFormContext, useWatch } from 'react-hook-form'

import { EditorSection } from '@/features/quotes/editor/EditorSection'
import { formatDefaultNetTerms } from '@/lib/mapping/priceToSubscriptionFee'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { TrialConfig } from '@/rpc/api/plans/v1/models_pb'
import type { ReactNode } from 'react'

const Hint = ({ children }: { children: ReactNode }) => (
  <TooltipProvider>
    <Tooltip>
      <TooltipTrigger asChild>
        <InfoIcon className="h-4 w-4 cursor-help text-muted-foreground" />
      </TooltipTrigger>
      <TooltipContent multiline className="max-w-xs">
        {children}
      </TooltipContent>
    </Tooltip>
  </TooltipProvider>
)

export const PAYMENT_LABELS: Record<QuoteFormValues['payment_methods_type'], string> = {
  online: 'Online',
  bankTransfer: 'Bank transfer',
  external: 'External',
}

interface TermsSectionProps {
  planVersionId?: string
  trialConfig?: TrialConfig
  trialingPlanName?: string
  isFreePlan: boolean
  hasOnlinePaymentProvider: boolean
  isLoadingProviders: boolean
  /** Start and end dates belong to one deal: a template has none. */
  hideDates?: boolean
}

/** How far after a fixed start the quick end dates fall. */
const END_PRESET_YEARS = [1, 2, 3]

/** A label and a control side by side under one heading: a choice, then its date if it has one. */
const ChoiceRow = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="flex flex-col @md:col-span-2">
    <span className="my-2 text-xs text-muted-foreground">{label}</span>
    <div className="grid grid-cols-1 gap-2 @md:grid-cols-2">{children}</div>
  </div>
)

/**
 * How the subscription comes about and when it runs. One question at a time: is it created when
 * the quote is signed or by hand afterwards; does it start then or on a set date; does it end.
 * An empty start date means "the day the subscription is created", which with automatic
 * creation is the day of signature: the choices say so instead of leaving a blank field.
 */
export const TermsSection = ({
  planVersionId,
  trialConfig,
  trialingPlanName,
  isFreePlan,
  hasOnlinePaymentProvider,
  isLoadingProviders,
  hideDates = false,
}: TermsSectionProps) => {
  const { control, setValue } = useFormContext<QuoteFormValues>()
  const [createOnAcceptance, startDate, endDate] = useWatch({
    control,
    name: ['create_subscription_on_acceptance', 'start_date', 'end_date'],
  })
  const noProvider = !isLoadingProviders && !hasOnlinePaymentProvider

  // "On a date" with no date picked yet is a choice the form values can't hold: kept here.
  const [startOnDate, setStartOnDate] = useState(() => Boolean(startDate))
  const [endOnDate, setEndOnDate] = useState(() => Boolean(endDate))
  // A date set from elsewhere (a restored draft, an applied template) shows as "On a date".
  if (startDate && !startOnDate) setStartOnDate(true)
  if (endDate && !endOnDate) setEndOnDate(true)

  const setField = (name: 'start_date' | 'end_date', value: Date | undefined) =>
    setValue(name, value, { shouldDirty: true, shouldValidate: true })

  const createdLabel = createOnAcceptance ? 'On signature' : 'When you create it'
  const endPresets = startDate
    ? END_PRESET_YEARS.map(years => ({
        label: years === 1 ? '1 year after start' : `${years} years after start`,
        date: addYears(startDate, years),
      }))
    : undefined

  const trialDescription = trialConfig?.durationDays
    ? `Plan default: ${trialConfig.durationDays} days${trialingPlanName ? ` with "${trialingPlanName}" features` : ''}. ${
        isFreePlan
          ? 'Free plan, no payment required.'
          : trialConfig.trialIsFree
            ? 'Free trial: payment method collected at checkout, billing starts after the trial.'
            : 'Paid trial: billing starts immediately.'
      } Empty uses the plan default, 0 skips the trial.`
    : undefined

  return (
    <EditorSection id="quote-terms" title="Subscription terms">
      <div className="grid grid-cols-1 gap-x-4 gap-y-2 @md:grid-cols-2">
        <GenericFormField
          control={control}
          layout="vertical"
          label="Subscription"
          name="create_subscription_on_acceptance"
          containerClassName="@md:col-span-2"
          labelTooltip={
            <Hint>
              Automatically: the subscription is created once every recipient has signed. Manually:
              you create it from the accepted quote yourself.
            </Hint>
          }
          render={({ field }) => (
            <Select
              value={field.value ? 'auto' : 'manual'}
              onValueChange={value => field.onChange(value === 'auto')}
            >
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="auto">Create automatically when signed</SelectItem>
                <SelectItem value="manual">Create manually afterwards</SelectItem>
              </SelectContent>
            </Select>
          )}
        />

        {!hideDates && (
          <>
            <ChoiceRow label="Starts">
              <Select
                value={startOnDate ? 'date' : 'created'}
                onValueChange={value => {
                  const onDate = value === 'date'
                  setStartOnDate(onDate)
                  if (!onDate) setField('start_date', undefined)
                }}
              >
                <SelectTrigger aria-label="When the subscription starts">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="created">{createdLabel}</SelectItem>
                  <SelectItem value="date">On a date</SelectItem>
                </SelectContent>
              </Select>
              {startOnDate && (
                <DatePicker
                  mode="single"
                  captionLayout="dropdown"
                  placeholder="Pick the start date"
                  date={startDate}
                  onSelect={date => setField('start_date', date)}
                  clearable={false}
                />
              )}
            </ChoiceRow>

            <ChoiceRow label="Ends">
              <Select
                value={endOnDate ? 'date' : 'never'}
                onValueChange={value => {
                  const onDate = value === 'date'
                  setEndOnDate(onDate)
                  if (!onDate) setField('end_date', undefined)
                }}
              >
                <SelectTrigger aria-label="When the subscription ends">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="never">No end date</SelectItem>
                  <SelectItem value="date">On a date</SelectItem>
                </SelectContent>
              </Select>
              {endOnDate && (
                <DatePicker
                  mode="single"
                  captionLayout="dropdown"
                  placeholder="Pick the end date"
                  date={endDate}
                  onSelect={date => setField('end_date', date)}
                  presets={endPresets}
                  clearable={false}
                />
              )}
            </ChoiceRow>
          </>
        )}

        {planVersionId && trialConfig?.durationDays ? (
          <InputFormField
            name="trial_duration"
            label="Trial (days)"
            type="number"
            min={0}
            placeholder={String(trialConfig.durationDays)}
            control={control}
            labelTooltip={trialDescription ? <Hint>{trialDescription}</Hint> : undefined}
            layout="vertical"
          />
        ) : (
          <div className="flex flex-col">
            <span className="my-2 text-xs text-muted-foreground">Trial</span>
            <span className="flex h-9 items-center text-sm text-muted-foreground">
              {planVersionId ? 'None' : '—'}
            </span>
          </div>
        )}
        <InputFormField
          name="billing_day_anchor"
          label="Billing day"
          control={control}
          type="number"
          min={1}
          max={31}
          placeholder="Anniversary"
          layout="vertical"
        />
        <SelectFormField
          name="activation_condition"
          label="Activation"
          placeholder="Select when to activate"
          control={control}
          layout="vertical"
          labelTooltip={
            <Hint>
              When the created subscription becomes active: on its start date, once the customer
              completes checkout, or when you activate it.
            </Hint>
          }
        >
          <SelectItem value="ON_START">On start date</SelectItem>
          <SelectItem value="ON_CHECKOUT" disabled={noProvider}>
            On checkout{noProvider && ' (requires a payment provider)'}
          </SelectItem>
          <SelectItem value="MANUAL">Manual activation</SelectItem>
        </SelectFormField>
      </div>
    </EditorSection>
  )
}

interface PaymentSectionProps {
  netTermsDefault?: { netTerms: number; sellsCredit: boolean }
  hasOnlinePaymentProvider: boolean
  isLoadingProviders: boolean
}

/** How the customer pays and how the subscription's invoices go out. */
export const PaymentSection = ({
  netTermsDefault,
  hasOnlinePaymentProvider,
  isLoadingProviders,
}: PaymentSectionProps) => {
  const { control } = useFormContext<QuoteFormValues>()
  const [activationCondition, paymentMethodsType] = useWatch({
    control,
    name: ['activation_condition', 'payment_methods_type'],
  })

  const noProvider = !isLoadingProviders && !hasOnlinePaymentProvider
  const offlinePayment = paymentMethodsType === 'bankTransfer' || paymentMethodsType === 'external'
  const chargeDisabled = noProvider || offlinePayment

  return (
    <EditorSection id="quote-payment" title="Payment & invoicing">
      <div className="grid grid-cols-1 gap-x-4 gap-y-2 @md:grid-cols-2">
        <SelectFormField
          name="payment_methods_type"
          label="Payment methods"
          placeholder="Select payment method type"
          control={control}
          layout="vertical"
          labelTooltip={
            <Hint>
              Online: card and/or direct debit through your payment providers.
              <br />
              Bank transfer: invoices carry bank transfer instructions.
              <br />
              External: payment is collected outside Meteroid.
            </Hint>
          }
        >
          <SelectItem value="online">Online (card / direct debit)</SelectItem>
          <SelectItem value="bankTransfer" disabled={activationCondition === 'ON_CHECKOUT'}>
            Bank transfer
            {activationCondition === 'ON_CHECKOUT' && ' (unavailable with Checkout)'}
          </SelectItem>
          <SelectItem value="external" disabled={activationCondition === 'ON_CHECKOUT'}>
            External
            {activationCondition === 'ON_CHECKOUT' && ' (unavailable with Checkout)'}
          </SelectItem>
        </SelectFormField>
        <InputFormField
          name="net_terms"
          label="Net terms (days)"
          control={control}
          type="number"
          min={0}
          placeholder={netTermsDefault ? formatDefaultNetTerms(netTermsDefault) : undefined}
          labelTooltip={<Hint>0 is due on receipt. Empty applies the default.</Hint>}
          layout="vertical"
        />
      </div>

      <div className="flex flex-wrap items-center gap-x-6 gap-y-3">
        <GenericFormField
          control={control}
          name="auto_advance_invoices"
          render={({ field }) => (
            <div className="flex items-center gap-2">
              <Switch
                id="autoAdvanceInvoices"
                checked={field.value}
                onCheckedChange={field.onChange}
              />
              <Label htmlFor="autoAdvanceInvoices" className="text-sm font-normal">
                Auto-advance invoices
              </Label>
              <Hint>
                Finalize draft invoices automatically when generated. Disable to review and finalize
                them manually.
              </Hint>
            </div>
          )}
        />
        <GenericFormField
          control={control}
          name="charge_automatically"
          render={({ field }) => (
            <div className="flex items-center gap-2">
              <Switch
                id="chargeAutomatically"
                checked={field.value}
                onCheckedChange={field.onChange}
                disabled={chargeDisabled}
              />
              <Label
                htmlFor="chargeAutomatically"
                className={`text-sm font-normal ${chargeDisabled ? 'text-muted-foreground' : ''}`}
              >
                Charge automatically
              </Label>
              <Hint>
                {!hasOnlinePaymentProvider
                  ? 'Requires a card or direct debit payment provider on the invoicing entity.'
                  : offlinePayment
                    ? 'Not available with Bank transfer or External payment methods. Switch to Online to enable.'
                    : 'Try charging the customer when an invoice is finalized, if a payment method is configured.'}
              </Hint>
            </div>
          )}
        />
      </div>

      <TextareaFormField
        name="invoice_memo"
        label="Invoice memo"
        control={control}
        placeholder="Custom note for invoices"
        layout="vertical"
        rows={2}
      />
    </EditorSection>
  )
}
