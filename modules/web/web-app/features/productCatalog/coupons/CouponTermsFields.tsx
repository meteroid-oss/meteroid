import {
  Button,
  Calendar,
  CheckboxFormField,
  FormControl,
  GenericFormField,
  Input,
  InputFormField,
  MultiSelectFormField,
  MultiSelectItem,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectFormField,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@ui/components'
import { cn } from '@ui/lib'
import { format } from 'date-fns'
import { CalendarIcon, PlusIcon, Trash2Icon, XIcon } from 'lucide-react'
import { useController, useFieldArray, useWatch } from 'react-hook-form'

import { CurrencySelect } from '@/components/CurrencySelect'
import { AdvancedSection, useHasFieldErrors } from '@/components/form'
import { EligibilityEditor } from '@/features/eligibility/EligibilityEditor'
import { fromDraft } from '@/features/eligibility/eligibility'
import { Segmented } from '@/features/plans/charges/ChargeComposer'
import { couponTermsSummary } from '@/features/productCatalog/coupons/couponTerms'
import { CREDIT_COUPON_HINT } from '@/features/productCatalog/coupons/discountLabel'
import { getCountryName } from '@/features/settings/utils'
import { useQuery } from '@/lib/connectrpc'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

import type { AmountRow } from './couponTerms'
import type { EligibilityDraft } from '@/features/eligibility/eligibility'
import type { Control } from 'react-hook-form'

export type DiscountKind = 'percentage' | 'fixed' | 'credit'

/** The fields the create and edit forms of a coupon share. */
export interface CouponTermsFormFields {
  discountType?: DiscountKind
  percentage?: number
  amount?: number
  creditAmount?: string
  currency?: string
  duration?: 'once' | 'forever' | 'months'
  durationMonths?: number
  otherAmounts?: AmountRow[]
  addOnIds?: string[]
  validFrom?: Date
  expiresAt?: Date
  redemptionLimit?: number
  reusable?: boolean
  creditExpiresInDays?: number
  creditSubscriptionOnly?: boolean
}

type FieldsControl = { control: Control<CouponTermsFormFields> }

/** The same fixed discount in other currencies: a subscription in none of them can't redeem it. */
export const OtherAmountsField = ({ control }: FieldsControl) => {
  const { fields, append, remove } = useFieldArray({ control, name: 'otherAmounts' })
  const main = useWatch({ control, name: 'currency' })
  const currencies = (useQuery(listTenantCurrencies).data?.currencies ?? []).filter(c => c !== main)
  if (currencies.length === 0 && fields.length === 0) return null

  return (
    <div className="grid gap-2 md:grid-cols-12">
      <span className="text-xs text-muted-foreground md:col-span-4 md:pt-2.5">
        In other currencies
      </span>
      <div className="space-y-2 md:col-span-8">
        {fields.map((field, index) => (
          <div key={field.id} className="flex items-start gap-2">
            <SelectFormField
              name={`otherAmounts.${index}.currency`}
              control={control}
              placeholder="Currency"
              className="w-28 font-mono"
              containerClassName="w-28"
            >
              {currencies.map(code => (
                <SelectItem key={code} value={code} className="font-mono">
                  {code}
                </SelectItem>
              ))}
            </SelectFormField>
            <InputFormField
              name={`otherAmounts.${index}.amount`}
              control={control}
              type="text"
              inputMode="decimal"
              placeholder="0.00"
              containerClassName="flex-1"
            />
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="size-9 shrink-0"
              aria-label="Remove this currency"
              onClick={() => remove(index)}
            >
              <Trash2Icon size={14} />
            </Button>
          </div>
        ))}
        <Button
          type="button"
          variant="ghost"
          size="sm"
          hasIcon
          className="h-8 text-xs"
          onClick={() => append({ currency: '', amount: '' })}
        >
          <PlusIcon size={12} /> Currency
        </Button>
      </div>
    </div>
  )
}

/** A day the coupon starts or ends on; empty: no bound. */
export const CouponDateField = ({
  control,
  name,
  label,
  empty,
  disablePast,
}: FieldsControl & {
  name: 'validFrom' | 'expiresAt'
  label: string
  empty: string
  disablePast?: boolean
}) => (
  <GenericFormField
    control={control}
    layout="horizontal"
    name={name}
    label={label}
    render={({ field }) => (
      <div className="col-span-8 flex items-center gap-1">
        <Popover>
          <PopoverTrigger asChild>
            <FormControl>
              <Button
                variant="outline"
                className={cn(
                  'w-full pl-3 text-left font-normal',
                  !field.value && 'text-muted-foreground'
                )}
              >
                {field.value ? format(field.value, 'PPP') : <span>{empty}</span>}
                <CalendarIcon className="ml-auto h-4 w-4 opacity-50" />
              </Button>
            </FormControl>
          </PopoverTrigger>
          <PopoverContent className="w-auto p-0" align="start">
            <Calendar
              mode="single"
              selected={field.value}
              onSelect={field.onChange}
              disabled={disablePast ? date => date < new Date() : undefined}
              autoFocus
            />
          </PopoverContent>
        </Popover>
        {field.value && (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-9 shrink-0"
            aria-label={`Clear ${label.toLowerCase()}`}
            onClick={() => field.onChange(undefined)}
          >
            <XIcon size={14} />
          </Button>
        )}
      </div>
    )}
  />
)

/** The add-ons the discount applies to, beside the plans; both empty: everything. */
export const CouponScopeFields = ({ control }: FieldsControl) => {
  const addOns =
    useQuery(listAddOns, { pagination: { perPage: 100, page: 0 } }).data?.addOns ?? []
  return (
    <MultiSelectFormField
      name="addOnIds"
      label="Add-ons"
      layout="horizontal"
      control={control}
      placeholder="All add-ons"
      hasSearch
    >
      {addOns.map(addOn => (
        <MultiSelectItem key={addOn.id} value={addOn.id}>
          {addOn.name}
        </MultiSelectItem>
      ))}
    </MultiSelectFormField>
  )
}

const DISCOUNT_KINDS: { value: DiscountKind; label: string }[] = [
  { value: 'percentage', label: '% off' },
  { value: 'fixed', label: 'Amount off' },
  { value: 'credit', label: 'Credit' },
]

/** The discount on one row: its kind, then its value. `locked`: kinds it can't switch to. */
export const CouponDiscountField = ({
  control,
  locked = [],
}: FieldsControl & { locked?: DiscountKind[] }) => (
  <GenericFormField
    control={control}
    name="discountType"
    label="Discount"
    render={({ field }) => (
      <div className="space-y-1">
        <div className="flex flex-wrap items-start gap-2">
          <div className="flex h-9 items-center">
            <Segmented<DiscountKind>
              label="Discount"
              value={field.value ?? 'percentage'}
              options={DISCOUNT_KINDS.map(k => ({ ...k, disabled: locked.includes(k.value) }))}
              onChange={field.onChange}
            />
          </div>
          {field.value === 'percentage' && (
            <InputFormField
              name="percentage"
              control={control}
              type="number"
              placeholder="0"
              rightText="%"
              aria-label="Percentage off"
              containerClassName="w-28"
            />
          )}
          {field.value === 'fixed' && (
            <>
              <InputFormField
                name="amount"
                control={control}
                type="number"
                placeholder="0.00"
                aria-label="Amount off"
                containerClassName="w-28"
              />
              <CurrencySelect
                name="currency"
                label=""
                layout="vertical"
                control={control}
                className="w-24"
              />
            </>
          )}
          {field.value === 'credit' && (
            <>
              <InputFormField
                name="creditAmount"
                control={control}
                type="text"
                inputMode="decimal"
                placeholder="0.00"
                aria-label="Credit amount"
                containerClassName="w-28"
              />
              <CurrencySelect
                name="currency"
                label=""
                layout="vertical"
                control={control}
                className="w-24"
              />
            </>
          )}
        </div>
        {field.value === 'credit' && (
          <p className="text-xs text-muted-foreground">{CREDIT_COUPON_HINT}</p>
        )}
      </div>
    )}
  />
)

/** How long it discounts a subscription. `withOnce`: only a new coupon offers the first invoice. */
export const CouponDurationField = ({
  control,
  withOnce = false,
  note,
}: FieldsControl & { withOnce?: boolean; note?: string }) => {
  const months = useController({ control, name: 'durationMonths' })
  return (
    <GenericFormField
      control={control}
      name="duration"
      label="Duration"
      render={({ field }) => (
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <Select value={field.value} onValueChange={field.onChange}>
              <SelectTrigger className="w-40">
                <SelectValue>
                  {field.value === 'months' ? 'For' : field.value === 'once' ? 'Once' : 'Forever'}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                {withOnce && <SelectItem value="once">Once</SelectItem>}
                <SelectItem value="forever">Forever</SelectItem>
                <SelectItem value="months">For N months</SelectItem>
              </SelectContent>
            </Select>
            {field.value === 'months' && (
              <>
                <Input
                  ref={months.field.ref}
                  name={months.field.name}
                  value={months.field.value ?? ''}
                  onChange={e =>
                    months.field.onChange(e.target.value === '' ? undefined : Number(e.target.value))
                  }
                  onBlur={months.field.onBlur}
                  type="number"
                  min={1}
                  max={120}
                  inputMode="numeric"
                  placeholder="12"
                  aria-label="Months"
                  className="w-20"
                />
                <span className="text-sm text-muted-foreground">months</span>
              </>
            )}
          </div>
          {months.fieldState.error && (
            <p className="text-xs text-destructive">{months.fieldState.error.message}</p>
          )}
          {field.value === 'months' && (
            <p className="text-xs text-muted-foreground">
              Billing periods starting within{' '}
              {months.field.value === 1 ? '1 month' : `${months.field.value || 'N'} months`} of
              the first paid period.
            </p>
          )}
          {note && <p className="text-xs text-muted-foreground">{note}</p>}
        </div>
      )}
    />
  )
}

/**
 * Everything a coupon holds beyond its discount, duration and plans, folded: closed unless one is
 * set, read in one line while closed. `creating`: the limits are set only then, never in the past.
 */
export const MoreConditions = ({
  control,
  kind,
  creating = false,
  eligibility,
  onEligibilityChange,
  defaultOpen,
}: FieldsControl & {
  kind: DiscountKind | undefined
  creating?: boolean
  eligibility: EligibilityDraft
  onEligibilityChange: (value: EligibilityDraft) => void
  defaultOpen?: boolean
}) => {
  const [
    validFrom,
    expiresAt,
    redemptionLimit,
    reusable,
    otherAmounts,
    creditExpiresInDays,
    creditSubscriptionOnly,
    addOnIds,
  ] = useWatch({
    control,
    name: [
      'validFrom',
      'expiresAt',
      'redemptionLimit',
      'reusable',
      'otherAmounts',
      'creditExpiresInDays',
      'creditSubscriptionOnly',
      'addOnIds',
    ],
  })
  const invalid = useHasFieldErrors(control, [
    'validFrom',
    'expiresAt',
    'redemptionLimit',
    'reusable',
    'otherAmounts',
    'creditExpiresInDays',
    'creditSubscriptionOnly',
    'addOnIds',
  ])
  const credit = kind === 'credit'
  const summary = couponTermsSummary(
    {
      validFrom,
      expiresAt,
      redemptionLimit: creating ? redemptionLimit : undefined,
      reusable: creating && !credit ? reusable : undefined,
      otherAmounts: kind === 'fixed' ? otherAmounts : undefined,
      creditExpiresInDays: credit ? creditExpiresInDays : undefined,
      creditSubscriptionOnly: credit ? creditSubscriptionOnly : undefined,
      addOnCount: addOnIds?.length,
      eligibility: fromDraft(eligibility),
    },
    code => getCountryName(code)
  )

  return (
    <AdvancedSection
      label="More conditions"
      summary={summary}
      defaultOpen={defaultOpen ?? summary !== ''}
      forceOpen={invalid}
    >
      <CouponDateField
        control={control}
        name="validFrom"
        label="Valid from"
        empty="As soon as it is created"
      />
      <CouponDateField
        control={control}
        name="expiresAt"
        label="Expiration"
        empty="No expiration"
        disablePast={creating}
      />
      {creating && (
        <InputFormField
          name="redemptionLimit"
          label="Max redemptions"
          layout="horizontal"
          control={control}
          type="number"
          placeholder="Unlimited"
          inputMode="numeric"
          description="Across all customers."
        />
      )}
      {creating && !credit && (
        <CheckboxFormField
          name="reusable"
          label="Reusable"
          control={control}
          layout="horizontal"
          description="The same customer can use it on several subscriptions."
        />
      )}
      {kind === 'fixed' && <OtherAmountsField control={control} />}
      {credit && (
        <>
          <InputFormField
            name="creditExpiresInDays"
            label="Credit expires after (days)"
            layout="horizontal"
            control={control}
            type="number"
            min={1}
            max={3660}
            placeholder="Never"
            inputMode="numeric"
          />
          <CheckboxFormField
            name="creditSubscriptionOnly"
            label="Only the subscription it is redeemed on"
            control={control}
            layout="horizontal"
            description="The credit ends with that subscription."
          />
        </>
      )}
      <CouponScopeFields control={control} />
      <div className="grid gap-2 md:grid-cols-12">
        <span className="text-xs text-muted-foreground md:col-span-4 md:pt-0.5">
          Who can redeem it
        </span>
        <div className="md:col-span-8">
          <EligibilityEditor value={eligibility} onChange={onEligibilityChange} />
        </div>
      </div>
    </AdvancedSection>
  )
}
