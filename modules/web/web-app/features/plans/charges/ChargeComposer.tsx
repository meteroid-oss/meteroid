import { skipToken } from '@connectrpc/connect-query'
import { GenericFormField, InputFormField, SelectFormField, SelectItem, cn } from '@md/ui'
import {
  createContext,
  useContext,
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react'
import { useController, useFormContext, useWatch } from 'react-hook-form'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { usePlanOverview } from '@/features/plans/hooks/usePlan'
import { MetricCombobox, MetricDisplay } from '@/features/plans/pricecomponents/StructuralFields'
import { feeTypeToHuman } from '@/features/plans/pricecomponents/utils'
import {
  CreditUnitPriceFields,
  useSelectedCreditUnit,
} from '@/features/pricing/CreditUnitPriceFields'
import { PricingFields, RateCardPricingFields } from '@/features/pricing/PricingFields'
import { NewRateCardDialog } from '@/features/productCatalog/ratecards/NewRateCardDialog'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'

import { QuantityField } from './QuantityField'
import {
  MODEL_LABELS,
  PRESETS,
  SOURCE_LABELS,
  bestFit,
  billingsFor,
  modelsFor,
  normalizeShape,
  presetOf,
  recurrencesFor,
  valuesForShape,
} from './compile'
import { singular, summarizeCharge } from './summary'

/** "seat" or "Seats" → "seats". */
const pluralUnit = (name: string) => {
  const one = singular(name).toLowerCase()
  return one.endsWith('s') ? one : `${one}s`
}

import type {
  Billing,
  ChargeLock,
  ChargePreset,
  ChargeShape,
  ChargeValues,
  QuantitySource,
  RateSource,
  RatingModel,
  Recurrence,
} from './compile'
import type { StructuralInfo } from '@/features/pricing/formData'
import type { DimensionRateRow } from '@/features/pricing/schemas'
import type { PriceRates } from '@/features/productCatalog/ratecards/bridge'
import type { Methods } from '@/hooks/useZodForm'
import type { Control, FieldValues, UseFormSetValue } from 'react-hook-form'
import type { z } from 'zod'

interface ChargeComposerProps {
  shape: ChargeShape
  onShapeChange: (shape: ChargeShape) => void
  methods: Methods<z.ZodType>
  currency: string
  /** Set for a charge of fixed shape: only shapes compiling to it are offered. */
  lock?: ChargeLock
  /** The metric and seat label of a locked charge, shown read-only. */
  structural?: StructuralInfo
  familyLocalId?: string
  /** "Include without price" is offered when set (existing products only). */
  include?: boolean
  onIncludeChange?: (include: boolean) => void
}

export const ChargeComposer = ({
  shape,
  onShapeChange,
  methods,
  currency,
  lock,
  structural,
  familyLocalId: familyLocalIdProp,
  include,
  onIncludeChange,
}: ChargeComposerProps) => {
  const plan = usePlanOverview()
  const familyLocalId = familyLocalIdProp ?? plan?.productFamilyLocalId
  const creditPricing = useEntitlement(entitlementCodes.CREDIT_PRICING)
  const control = methods.control as unknown as Control<FieldValues>
  const values = (useWatch({ control }) ?? {}) as ChargeValues

  const applyShape = (next: ChargeShape) => {
    const normalized = normalizeShape(next)
    const nextValues = valuesForShape(normalized, methods.getValues() as ChargeValues, shape)
    if (normalized.model === 'credit_pack') nextValues.once = normalized.recurrence === 'one_time'
    onShapeChange(normalized)
    methods.reset(nextValues)
    onIncludeChange?.(false)
  }

  const answer = (change: Partial<ChargeShape>) => {
    const next = bestFit(shape, change, lock)
    if (next) applyShape(next)
  }
  const offered = (change: Partial<ChargeShape>) => bestFit(shape, change, lock) !== undefined

  // A credit pack's own "Grant" switch and the When answer are one choice.
  const once = values.once as boolean | undefined
  useEffect(() => {
    if (shape.model !== 'credit_pack' || once === undefined) return
    const recurrence: Recurrence = once ? 'one_time' : 'recurring'
    if (recurrence !== shape.recurrence) onShapeChange({ ...shape, recurrence })
  }, [once, shape, onShapeChange])

  const metricId = (values.metricId as string | undefined) || structural?.metricId
  const metricsQuery = useQuery(
    listBillableMetrics,
    shape.source === 'metered' ? (familyLocalId ? { familyLocalId } : {}) : skipToken
  )
  const metricName = metricsQuery.data?.billableMetrics?.find(m => m.id === metricId)?.name
  const creditUnit = useSelectedCreditUnit(control)

  const activePreset = include ? 'include' : presetOf(shape)
  const presets = PRESETS.filter(
    preset =>
      (preset.id !== 'include' || onIncludeChange) &&
      (preset.id !== 'credit' || creditPricing || shape.model === 'credit_pack')
  )
  const presetOffered = (preset: ChargePreset) =>
    preset.shape ? bestFit(preset.shape, preset.shape, lock) !== undefined : true

  const models = modelsFor(shape.source).filter(
    model => model === shape.model || model !== 'credit_pack' || creditPricing
  )
  const recurrences = recurrencesFor(shape)
  const billings = billingsFor(shape)
  const newProduct = !lock

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap gap-1.5">
        {presets.map(preset => {
          const enabled = presetOffered(preset)
          return (
            <button
              key={preset.id}
              type="button"
              disabled={!enabled}
              aria-pressed={activePreset === preset.id}
              onClick={() =>
                preset.shape ? applyShape(preset.shape) : onIncludeChange?.(!include)
              }
              className={cn(
                'rounded-full border px-3 py-1 text-xs font-medium transition-colors',
                activePreset === preset.id
                  ? 'border-brand bg-brand/10 text-foreground'
                  : 'border-border text-muted-foreground hover:text-foreground hover:border-muted-foreground/40',
                'disabled:cursor-not-allowed disabled:opacity-40'
              )}
            >
              {preset.label}
            </button>
          )
        })}
      </div>

      {lock && (
        <p className="text-xs text-muted-foreground">
          This charge bills as “{feeTypeToHuman(lock.feeType)}”: another kind is another charge.
        </p>
      )}

      {include ? (
        <p className="rounded-md border border-border bg-muted/40 px-3 py-2 text-sm">
          Adds the product with its features and no price.
        </p>
      ) : (
        <>
          <div className="rounded-lg border border-border divide-y divide-border">
            <Question label="How many">
              <Segmented<QuantitySource>
                value={shape.source}
                options={(['fixed', 'seats', 'metered'] as QuantitySource[]).map(source => ({
                  value: source,
                  label: SOURCE_LABELS[source],
                  disabled: !offered({ source }),
                }))}
                onChange={source => answer({ source })}
              />
              {shape.source === 'fixed' &&
                shape.model !== 'flat' &&
                shape.model !== 'credit_pack' && (
                  <InputFormField
                    name="quantity"
                    label="Quantity"
                    type="number"
                    step={1}
                    min={1}
                    className="max-w-[140px]"
                    control={control}
                  />
                )}
              {shape.source === 'seats' &&
                (newProduct ? (
                  <QuantityField
                    control={control}
                    setValue={methods.setValue as unknown as UseFormSetValue<FieldValues>}
                    quantityId={values.quantityId as string | undefined}
                  />
                ) : (
                  <p className="text-xs text-muted-foreground">
                    Counted in {pluralUnit(structural?.slotUnitName || 'seat')}
                  </p>
                ))}
              {shape.source === 'metered' &&
                (newProduct ? (
                  <MetricCombobox methods={methods} familyLocalId={familyLocalId} />
                ) : (
                  <MetricDisplay metricId={structural?.metricId} familyLocalId={familyLocalId} />
                ))}
            </Question>

            <Question label="How much">
              <Segmented<RatingModel>
                value={shape.model}
                options={models.map(model => ({
                  value: model,
                  label: MODEL_LABELS[model],
                  disabled: !offered({ model }),
                }))}
                onChange={model => answer({ model })}
              />
            </Question>

            <Question label="When">
              <div className="flex flex-wrap items-center gap-3">
                {recurrences.length > 1 && (
                  <Segmented<Recurrence>
                    value={shape.recurrence}
                    options={recurrences.map(recurrence => ({
                      value: recurrence,
                      label:
                        recurrence === 'recurring'
                          ? 'Recurring'
                          : shape.model === 'credit_pack'
                            ? 'Once at start'
                            : 'One-time',
                      disabled: !offered({ recurrence }),
                    }))}
                    onChange={recurrence => answer({ recurrence })}
                  />
                )}
                {shape.recurrence === 'recurring' && (
                  <SelectFormField name="term" control={control} className="w-[140px]">
                    <SelectItem value="MONTHLY">Every month</SelectItem>
                    <SelectItem value="QUARTERLY">Every quarter</SelectItem>
                    <SelectItem value="SEMIANNUAL">Every 6 months</SelectItem>
                    <SelectItem value="ANNUAL">Every year</SelectItem>
                  </SelectFormField>
                )}
                {billings.length > 1 ? (
                  <Segmented<Billing>
                    label="Billing"
                    value={shape.billing}
                    options={billings.map(billing => ({
                      value: billing,
                      label: billing === 'advance' ? 'In advance' : 'In arrears',
                      disabled: !offered({ billing }),
                    }))}
                    onChange={billing => answer({ billing })}
                  />
                ) : (
                  <span className="text-xs text-muted-foreground">
                    {timingNote(shape)}
                  </span>
                )}
              </div>
            </Question>
          </div>

          <div key={`${shape.source}-${shape.model}`} className="space-y-4">
            <ChargePriceFields
              shape={shape}
              control={control}
              currency={currency}
              metricId={metricId}
              unitName={(values.slotUnitName as string | undefined) || structural?.slotUnitName}
              creditUnitCode={creditUnit?.code}
            />
          </div>

          <p
            role="status"
            className="rounded-md border border-border bg-muted/40 px-3 py-2 text-sm"
          >
            {summarizeCharge(
              shape,
              {
                ...values,
                slotUnitName: (values.slotUnitName as string | undefined) || structural?.slotUnitName,
              },
              { currency, metricName, creditUnitName: creditUnit?.pluralName }
            )}
          </p>
        </>
      )}
    </div>
  )
}

function timingNote(shape: ChargeShape): string {
  if (shape.recurrence === 'one_time') return 'On the first invoice'
  if (shape.model === 'included') return 'Fee in advance, overage in arrears'
  if (shape.model === 'credit_pack') return 'Billed in advance'
  return shape.billing === 'advance' ? 'Billed in advance' : 'Billed in arrears'
}

// --- Price fields of the chosen model ---

const MoneyField = ({
  control,
  name,
  label,
  currency,
}: {
  control: Control<FieldValues>
  name: string
  label: string
  currency: string
}) => (
  <GenericFormField
    control={control}
    name={name}
    label={label}
    render={({ field }) => (
      <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
    )}
  />
)

export const ChargePriceFields = ({
  shape,
  control,
  currency,
  metricId,
  unitName,
  creditUnitCode,
}: {
  shape: ChargeShape
  control: Control<FieldValues>
  currency: string
  metricId?: string
  unitName?: string
  creditUnitCode?: string
}) => {
  switch (shape.source) {
    case 'fixed':
      switch (shape.model) {
        case 'flat':
          return <MoneyField control={control} name="rate" label="Amount" currency={currency} />
        case 'per_unit':
          return (
            <MoneyField control={control} name="unitPrice" label="Price per unit" currency={currency} />
          )
        case 'credit_pack':
          return <PricingFields pricingType="credit" control={control} currency={currency} />
        case 'tiered':
        case 'volume':
        case 'package':
          return <PricingFields pricingType={shape.model} control={control} currency={currency} />
        default:
          return null
      }
    case 'seats':
      return (
        <>
          {shape.model === 'per_unit' ? (
            <MoneyField
              control={control}
              name="unitPrice"
              label={`Price per ${singular(unitName || 'Seats')}`}
              currency={currency}
            />
          ) : shape.model === 'tiered' || shape.model === 'volume' || shape.model === 'package' ? (
            <PricingFields pricingType={shape.model} control={control} currency={currency} />
          ) : null}
          <div className="grid max-w-md grid-cols-2 gap-4">
            <InputFormField
              name="minSlots"
              label="Minimum"
              type="number"
              placeholder="Optional"
              control={control}
            />
            <InputFormField
              name="maxSlots"
              label="Maximum"
              type="number"
              placeholder="Optional"
              control={control}
            />
          </div>
        </>
      )
    case 'metered': {
      const usageCurrency = creditUnitCode ?? currency
      if (shape.model === 'included') {
        return (
          <>
            <CreditUnitPriceFields control={control} currency={currency} overageOnly />
            <PricingFields
              pricingType="capacity"
              control={control}
              currency={currency}
              overageCurrency={creditUnitCode}
            />
          </>
        )
      }
      if (shape.model === 'by_dimension') {
        return <ByDimensionFields control={control} currency={currency} metricId={metricId} />
      }
      // An amount of money: never priced in credits.
      if (shape.model === 'percentage' || shape.model === 'dynamic') {
        return <PricingFields pricingType={shape.model} control={control} currency={currency} />
      }
      return (
        <>
          <CreditUnitPriceFields control={control} currency={currency} />
          {shape.model === 'per_unit' && (
            <PricingFields pricingType="perUnit" control={control} currency={usageCurrency} />
          )}
          {(shape.model === 'tiered' || shape.model === 'volume' || shape.model === 'package') && (
            <PricingFields pricingType={shape.model} control={control} currency={usageCurrency} />
          )}
        </>
      )
    }
  }
}

/**
 * Usage priced by its dimensions: at rates the plan holds, or at a shared rate card's. A card's
 * rates can be copied into the plan, and the plan's saved as a new card the price then uses.
 */
const ByDimensionFields = ({
  control,
  currency,
  metricId,
}: {
  control: Control<FieldValues>
  currency: string
  metricId?: string
}) => {
  const { getValues, setValue } = useFormContext()
  const { field } = useController({ control, name: 'rateSource' })
  const source = (field.value as RateSource | undefined) ?? 'plan'
  const creditUnit = useSelectedCreditUnit(control)
  const [saving, setSaving] = useState<PriceRates>()
  const set = (name: string, value: unknown) => setValue(name, value, { shouldDirty: true })

  // The card's rates become the price's own: its source switches to them.
  const copy = (rates: PriceRates) => {
    set('dimensions', rates.dimensions)
    set('keyRates', rates.keyRates)
    set('unitSize', rates.unitSize)
    set('onUnmatched', rates.onUnmatched)
    set('markup', '')
    field.onChange('plan')
  }
  const ownRates = (): PriceRates => ({
    dimensions: (getValues('dimensions') as string[] | undefined) ?? [],
    keyRates: (getValues('keyRates') as DimensionRateRow[] | undefined) ?? [],
    unitSize: String(getValues('unitSize') ?? '1'),
    onUnmatched: getValues('onUnmatched') === 'ignore' ? 'ignore' : 'hold',
  })
  const named = (rates: PriceRates) =>
    rates.dimensions.length > 0 &&
    rates.dimensions.every(d => d.trim()) &&
    rates.keyRates.some(row => row.rate?.trim())

  return (
    <>
      <div className="flex flex-wrap items-center gap-3">
        <Segmented<RateSource>
          label="Rates"
          value={source}
          options={[
            { value: 'plan', label: 'Rates in this plan' },
            { value: 'card', label: 'From a rate card' },
          ]}
          onChange={field.onChange}
        />
        {source === 'plan' && metricId && (
          <Gate perm="catalog:manage">
            <button
              type="button"
              onClick={() => {
                const rates = ownRates()
                if (named(rates)) setSaving(rates)
                else toast.error('Name its dimensions and set a rate first')
              }}
              className="text-xs font-medium text-foreground hover:underline"
            >
              Save as a rate card…
            </button>
          </Gate>
        )}
      </div>
      {source === 'card' ? (
        <RateCardPricingFields
          control={control}
          metricId={metricId}
          currency={currency}
          onCopy={copy}
        />
      ) : (
        <>
          <CreditUnitPriceFields control={control} currency={currency} />
          <PricingFields
            pricingType="dimensionRates"
            control={control}
            currency={creditUnit?.code ?? currency}
            metricId={metricId}
          />
        </>
      )}
      {saving && metricId && (
        <NewRateCardDialog
          metricId={metricId}
          creditUnitId={creditUnit?.id}
          from={saving}
          currency={currency}
          onClose={() => setSaving(undefined)}
          onCreated={card => {
            setSaving(undefined)
            // The price is rated by the new card, as the plan's rates were.
            set('rateCardId', card.id)
            set('markup', '')
            field.onChange('card')
          }}
        />
      )}
    </>
  )
}

// --- Layout atoms ---

// The question's label names the choices inside it.
const QuestionLabel = createContext<string | undefined>(undefined)

const Question = ({ label, children }: { label: string; children: ReactNode }) => {
  const id = useId()
  return (
    <div className="grid grid-cols-[96px_1fr] items-start gap-3 p-3">
      <span id={id} className="pt-1.5 text-xs font-medium text-muted-foreground">
        {label}
      </span>
      <QuestionLabel.Provider value={id}>
        <div className="space-y-3">{children}</div>
      </QuestionLabel.Provider>
    </div>
  )
}

interface SegmentedOption<T extends string> {
  value: T
  label: string
  disabled?: boolean
}

/** A radio group: one tab stop, arrow keys move and select. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  /** None yet: nothing is checked, the first enabled option takes the tab stop. */
  value: T | undefined
  options: SegmentedOption<T>[]
  onChange: (value: T) => void
  /** Overrides the enclosing question's label. */
  label?: string
}) {
  const labelledBy = useContext(QuestionLabel)
  const buttons = useRef<(HTMLButtonElement | null)[]>([])
  const enabled = (option: SegmentedOption<T>) => !option.disabled || option.value === value
  const current = options.findIndex(option => option.value === value)
  const tabStop = current >= 0 ? current : options.findIndex(enabled)

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const step =
      event.key === 'ArrowRight' || event.key === 'ArrowDown'
        ? 1
        : event.key === 'ArrowLeft' || event.key === 'ArrowUp'
          ? -1
          : 0
    let target = -1
    if (event.key === 'Home') target = options.findIndex(enabled)
    else if (event.key === 'End') target = options.map(enabled).lastIndexOf(true)
    else if (step !== 0) {
      for (let i = 1; i <= options.length; i++) {
        const index = (tabStop + step * i + options.length) % options.length
        if (enabled(options[index])) {
          target = index
          break
        }
      }
    } else return
    event.preventDefault()
    if (target < 0 || target === tabStop) return
    buttons.current[target]?.focus()
    onChange(options[target].value)
  }

  return (
    <div
      role="radiogroup"
      aria-label={label}
      aria-labelledby={label ? undefined : labelledBy}
      onKeyDown={onKeyDown}
      className="inline-flex flex-wrap gap-0.5 rounded-md border border-border bg-muted p-0.5"
    >
      {options.map((option, index) => {
        const checked = option.value === value
        return (
          <button
            key={option.value}
            ref={element => {
              buttons.current[index] = element
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={index === tabStop ? 0 : -1}
            disabled={!enabled(option)}
            onClick={() => !checked && onChange(option.value)}
            className={cn(
              'rounded px-2.5 py-1 text-xs font-medium transition-colors',
              checked
                ? 'bg-background text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
              'disabled:cursor-not-allowed disabled:opacity-40'
            )}
          >
            {option.label}
          </button>
        )
      })}
    </div>
  )
}
