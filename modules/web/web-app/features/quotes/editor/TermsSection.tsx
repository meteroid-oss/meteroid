import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  DatePicker,
  GenericFormField,
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
import { ChevronRight, InfoIcon } from 'lucide-react'
import { useFormContext, useWatch } from 'react-hook-form'

import { EditorSection } from '@/features/quotes/editor/EditorSection'
import { formatDefaultNetTerms } from '@/lib/mapping/priceToSubscriptionFee'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { TrialConfig } from '@/rpc/api/plans/v1/models_pb'
import type { ReactNode } from 'react'

const Hint = ({ children }: { children: ReactNode }) => (
  <TooltipProvider delayDuration={100}>
    <Tooltip>
      <TooltipTrigger asChild>
        <InfoIcon className="h-4 w-4 cursor-help text-muted-foreground" />
      </TooltipTrigger>
      <TooltipContent className="max-w-xs">{children}</TooltipContent>
    </Tooltip>
  </TooltipProvider>
)

const PAYMENT_LABELS: Record<QuoteFormValues['payment_methods_type'], string> = {
  online: 'Online',
  bankTransfer: 'Bank transfer',
  external: 'External',
}

interface TermsSectionProps {
  planVersionId?: string
  trialConfig?: TrialConfig
  trialingPlanName?: string
  isFreePlan: boolean
  netTermsDefault?: { netTerms: number; sellsCredit: boolean }
  hasOnlinePaymentProvider: boolean
  isLoadingProviders: boolean
  /** Start and end dates belong to one deal: a template has none. */
  hideDates?: boolean
}

export const TermsSection = ({
  planVersionId,
  trialConfig,
  trialingPlanName,
  isFreePlan,
  netTermsDefault,
  hasOnlinePaymentProvider,
  isLoadingProviders,
  hideDates = false,
}: TermsSectionProps) => {
  const { control } = useFormContext<QuoteFormValues>()
  const [activationCondition, paymentMethodsType, autoAdvance, chargeAutomatically] = useWatch({
    control,
    name: [
      'activation_condition',
      'payment_methods_type',
      'auto_advance_invoices',
      'charge_automatically',
    ],
  })

  const noProvider = !isLoadingProviders && !hasOnlinePaymentProvider
  const offlinePayment = paymentMethodsType === 'bankTransfer' || paymentMethodsType === 'external'
  const chargeDisabled = noProvider || offlinePayment

  const paymentSummary = [
    PAYMENT_LABELS[paymentMethodsType] ?? PAYMENT_LABELS.online,
    autoAdvance ? 'auto-advance' : 'manual finalization',
    chargeAutomatically ? 'charge automatically' : 'no automatic charge',
  ].join(' · ')

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
    <EditorSection
      id="quote-terms"
      title="Subscription terms"
      aside={
        <GenericFormField
          control={control}
          name="create_subscription_on_acceptance"
          render={({ field }) => (
            <div className="flex items-center gap-2">
              <Switch
                id="createSubscriptionOnAcceptance"
                checked={field.value}
                onCheckedChange={field.onChange}
              />
              <Label htmlFor="createSubscriptionOnAcceptance" className="text-sm font-normal">
                Create the subscription on acceptance
              </Label>
              <Hint>
                When enabled, a subscription is created automatically once every recipient has
                signed the quote.
              </Hint>
            </div>
          )}
        />
      }
    >
      <div className="grid grid-cols-1 gap-x-4 gap-y-2 sm:grid-cols-2 xl:grid-cols-3">
        {!hideDates && (
          <>
            <GenericFormField
              control={control}
              layout="vertical"
              label="Starts"
              name="start_date"
              render={({ field }) => (
                <DatePicker
                  mode="single"
                  captionLayout="dropdown"
                  className="w-full"
                  placeholder="On creation"
                  date={field.value}
                  onSelect={field.onChange}
                  clearable
                />
              )}
            />
            <GenericFormField
              control={control}
              layout="vertical"
              label="Ends"
              name="end_date"
              render={({ field }) => (
                <DatePicker
                  mode="single"
                  captionLayout="dropdown"
                  className="w-full"
                  placeholder="No end date"
                  date={field.value}
                  onSelect={field.onChange}
                  clearable
                />
              )}
            />
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
        <SelectFormField
          name="activation_condition"
          label="Activation"
          placeholder="Select when to activate"
          control={control}
          layout="vertical"
        >
          <SelectItem value="ON_START">On start date</SelectItem>
          <SelectItem value="ON_CHECKOUT" disabled={noProvider}>
            On checkout{noProvider && ' (requires a payment provider)'}
          </SelectItem>
          <SelectItem value="MANUAL">Manual activation</SelectItem>
        </SelectFormField>
      </div>

      <Collapsible className="border-t border-border pt-3">
        <CollapsibleTrigger className="group flex w-full items-center gap-2 text-left">
          <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-90" />
          <span className="text-sm font-medium">Payment & invoicing</span>
          <span className="truncate text-xs text-muted-foreground">{paymentSummary}</span>
        </CollapsibleTrigger>
        <CollapsibleContent className="grid grid-cols-1 gap-x-4 gap-y-3 pt-3 md:grid-cols-2">
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

          <div className="flex flex-wrap items-end gap-x-6 gap-y-3 pb-1">
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
                    Finalize draft invoices automatically when generated. Disable to review and
                    finalize them manually.
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
            containerClassName="md:col-span-2"
          />
        </CollapsibleContent>
      </Collapsible>
    </EditorSection>
  )
}
