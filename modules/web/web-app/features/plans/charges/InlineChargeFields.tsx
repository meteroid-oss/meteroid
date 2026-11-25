import { skipToken } from '@connectrpc/connect-query'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  Label,
  cn,
} from '@md/ui'
import { ChevronDownIcon, ChevronRightIcon, PlusIcon } from 'lucide-react'
import { useId, useState, type ReactNode } from 'react'
import { useWatch } from 'react-hook-form'

import {
  formatPerUnit,
  metricUnitFromProto,
  unitPlural as pluralOfUnit,
} from '@/features/billing/metrics/units'
import {
  TERMS,
  termAdverb,
  termFormValue,
  termFromFormValue,
  termPer,
} from '@/features/plans/currencies/priceCells'
import { MetricCombobox } from '@/features/plans/pricecomponents/StructuralFields'
import { feeTypeToHuman } from '@/features/plans/pricecomponents/utils'
import { useSelectedCreditUnit } from '@/features/pricing/CreditUnitPriceFields'
import { currencySymbol } from '@/features/productCatalog/ratecards/rateTable'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'

import { ChargePriceFields, Segmented } from './ChargeComposer'
import { InvoicePreview } from './InvoicePreview'
import { LevelEditor } from './LevelEditor'
import { QuantityField } from './QuantityField'
import { Help, TierEditor } from './TierEditor'
import { bestFit, billingsFor, normalizeShape, shapeFits, valuesForShape } from './compile'
import {
  KIND_LABELS,
  MENU_KINDS,
  includedUsage,
  isAllowanceTiers,
  kindOffered,
  splitLastTier,
  unitIsCurrency,
  withIncludedUsage,
} from './kinds'
import { singular } from './summary'

import type {
  Billing,
  Cadence,
  ChargeLock,
  ChargeShape,
  ChargeValues,
  RatingModel,
  Recurrence,
} from './compile'
import type { ChargeKind, TierRowLike } from './kinds'
import type { MetricUnit } from '@/features/billing/metrics/units'
import type { Methods } from '@/hooks/useZodForm'
import type { BillableMetric } from '@/rpc/api/billablemetrics/v1/models_pb'
import type { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import type { Control, FieldValues, UseFormSetValue } from 'react-hook-form'
import type { z } from 'zod'

// --- Header parts: the name, what is edited, what it charges for ---

export const ChargeNameInput = ({
  value,
  onChange,
  placeholder = 'Charge name',
  autoFocus,
}: {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  autoFocus?: boolean
}) => {
  const id = useId()
  return (
    <div className="min-w-0 flex-1 space-y-1">
      <Label htmlFor={id} className="sr-only">
        Name
      </Label>
      <Input
        id={id}
        autoFocus={autoFocus}
        placeholder={placeholder}
        value={value}
        onChange={e => onChange(e.target.value)}
        className="h-9 rounded-none border-0 border-b border-dashed border-border bg-transparent px-0 text-base font-semibold shadow-none focus-visible:border-solid focus-visible:border-ring focus-visible:ring-0"
      />
    </div>
  )
}

/** The option and currency a form prices, as a quiet chip beside the name. */
export const ContextChip = ({ children }: { children: ReactNode }) => (
  <span className="inline-flex h-6 items-center rounded-full border border-border bg-muted/40 px-2.5 text-xs text-muted-foreground">
    {children}
  </span>
)

/** "What it charges for": the kinds, those a charge of fixed shape cannot become greyed out. */
export const KindPicker = ({
  kind,
  lock,
  unavailable = [],
  hidden = [],
  extra = [],
  onPick,
}: {
  kind?: ChargeKind
  lock?: ChargeLock
  unavailable?: ChargeKind[]
  /** Not offered at all here, e.g. "Included" outside a plan. */
  hidden?: ChargeKind[]
  /** Offered here beside the menu, e.g. "Credit pack" for an add-on. */
  extra?: ChargeKind[]
  onPick: (kind: ChargeKind) => void
}) => {
  const menu = [...MENU_KINDS, ...extra].filter(option => !hidden.includes(option))
  // A credit pack is not offered, but one being edited still reads as itself.
  const kinds = kind && !menu.includes(kind) ? [...menu, kind] : menu
  const options = kinds.map(option => ({
    value: option,
    label: KIND_LABELS[option],
    disabled: !kindOffered(option, lock) || unavailable.includes(option),
  }))
  const locked = lock && menu.some(option => !kindOffered(option, lock))
  return (
    <FieldRow label="What it charges for">
      <Segmented<ChargeKind>
        label="What it charges for"
        value={kind}
        options={options}
        onChange={next => next !== kind && onPick(next)}
      />
      {locked && lock && (
        <p className="text-[11px] text-muted-foreground">
          This charge bills as “{feeTypeToHuman(lock.feeType)}”: another kind is another charge.
        </p>
      )}
    </FieldRow>
  )
}

/** A labelled control of the form, its label above. */
const FieldRow = ({ label, children }: { label: ReactNode; children: ReactNode }) => (
  <div className="space-y-1.5">
    <span className="block text-xs text-muted-foreground">{label}</span>
    {children}
  </div>
)

/** The footer: `leading` on the left, removing then Cancel and the submit on the right. */
export const FormFooter = ({
  leading,
  remove,
  children,
}: {
  leading?: ReactNode
  remove?: { label: string; onClick: () => void; disabled?: boolean }
  children: ReactNode
}) => (
  <div className="flex flex-wrap items-center gap-2 pt-1">
    {leading}
    <div className="ml-auto flex items-center gap-3">
      {remove && (
        <button
          type="button"
          onClick={remove.onClick}
          disabled={remove.disabled}
          className="text-xs font-medium text-destructive hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:opacity-50"
        >
          {remove.label}
        </button>
      )}
      <div className="flex gap-2">{children}</div>
    </div>
  </div>
)

// --- One labelled field, its unit inside ---

const grouped = (value: unknown) => {
  const raw = String(value ?? '')
  if (raw === '') return ''
  const n = Number(raw)
  return Number.isFinite(n) ? n.toLocaleString('en-US') : raw
}
const digitsOnly = (raw: string) => raw.replace(/[^\d]/g, '')
const decimalOnly = (raw: string) => raw.replace(/[^\d.]/g, '')

type FieldKind = 'money' | 'count' | 'bigint'

export const UnitField = ({
  control,
  name,
  label,
  prefix,
  suffix,
  kind = 'money',
}: {
  control: Control<FieldValues>
  name: string
  label: string
  prefix?: string
  /** After the amount: its unit, or a control such as the cadence menu. */
  suffix?: ReactNode
  kind?: FieldKind
}) => (
  <FormField
    control={control}
    name={name}
    render={({ field, fieldState }) => (
      <FormItem className="space-y-1.5">
        <FormLabel className="text-xs font-normal text-muted-foreground">{label}</FormLabel>
        <div
          className={cn(
            'flex h-9 items-center gap-1.5 rounded-md border border-input bg-background px-2.5 text-sm shadow-sm focus-within:ring-1 focus-within:ring-ring',
            fieldState.error && 'border-destructive'
          )}
        >
          {prefix && <span className="text-sm text-muted-foreground">{prefix}</span>}
          <input
            ref={field.ref}
            name={field.name}
            inputMode={kind === 'money' ? 'decimal' : 'numeric'}
            aria-label={label}
            className="min-w-0 flex-1 bg-transparent font-mono tabular-nums outline-none"
            value={kind === 'money' ? String(field.value ?? '') : grouped(field.value)}
            onBlur={field.onBlur}
            onChange={e => {
              const raw = e.target.value
              if (kind === 'money') return field.onChange(decimalOnly(raw))
              const digits = digitsOnly(raw)
              if (kind === 'bigint') return field.onChange(BigInt(digits || '0'))
              return field.onChange(digits === '' ? undefined : Number(digits))
            }}
          />
          {suffix && <span className="shrink-0 text-xs text-muted-foreground">{suffix}</span>}
        </div>
        <FormMessage className="text-xs" />
      </FormItem>
    )}
  />
)

// --- The layout: the form on the left, what it puts on an invoice on the right ---

/**
 * The form beside its invoice preview, stacked where the container is narrow; the preview's
 * column keeps the form's height steady as fields come and go.
 */
export const ChargeLayout = ({
  preview,
  footer,
  children,
}: {
  preview: ReactNode
  footer: ReactNode
  children: ReactNode
}) => (
  <div className="@container space-y-4">
    <div className="grid items-start gap-x-6 gap-y-4 @3xl:grid-cols-[minmax(0,1fr)_19rem]">
      <div className="min-w-0 space-y-4">{children}</div>
      <div className="@3xl:sticky @3xl:top-0">{preview}</div>
    </div>
    {footer}
  </div>
)

/** How often a recurring charge bills, as a quiet menu beside its amount: "/ month ▾". */
const CadenceMenu = ({
  value,
  onChange,
  format = per => `/ ${per}`,
}: {
  value: BillingPeriod
  onChange: (cadence: Cadence) => void
  format?: (per: string) => string
}) => (
  <DropdownMenu>
    <DropdownMenuTrigger asChild>
      <button
        type="button"
        aria-label={`Billed every ${termPer(value)}`}
        className="inline-flex shrink-0 items-center gap-0.5 rounded px-1 text-xs text-muted-foreground hover:bg-accent hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
      >
        {format(termPer(value))}
        <ChevronDownIcon size={12} />
      </button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end">
      <DropdownMenuRadioGroup
        value={termFormValue(value)}
        onValueChange={next => onChange(next as Cadence)}
      >
        {TERMS.filter(t => t.param !== 'semiannual' || t.term === value).map(t => (
          <DropdownMenuRadioItem key={t.param} value={termFormValue(t.term)}>
            Every {t.per}
          </DropdownMenuRadioItem>
        ))}
      </DropdownMenuRadioGroup>
    </DropdownMenuContent>
  </DropdownMenu>
)

// --- The body: fields of the shape, its invoice preview, more options ---

export interface ChargeBodyContext {
  /** The metric of a charge of fixed shape; otherwise chosen in the form. */
  metric?: BillableMetric
  slotUnitName?: string
  metricId?: string
}

const MODEL_OPTIONS: Record<ChargeShape['source'], [RatingModel, string][]> = {
  fixed: [
    ['flat', 'One amount'],
    ['per_unit', 'Per unit'],
    ['tiered', 'Graduated'],
    ['volume', 'Volume'],
    ['package', 'Package'],
  ],
  seats: [
    ['per_unit', 'Per seat'],
    ['tiered', 'Graduated'],
    ['volume', 'Volume'],
    ['package', 'Package'],
  ],
  metered: [
    ['per_unit', 'Per unit'],
    ['tiered', 'Graduated'],
    ['volume', 'Volume'],
    ['package', 'Package'],
    ['by_dimension', 'By dimension'],
  ],
}

/** What the rating models mean, beside the "Priced" choice. */
const pricedHelp = (symbol: string) => (
  <span className="block space-y-1.5">
    <span className="block">
      <strong>Graduated</strong>: each tier prices only its own units. 1,500 units, the first 1,000
      at {symbol}1 and the rest at {symbol}0.50, cost {symbol}1,250.
    </span>
    <span className="block">
      <strong>Volume</strong>: every unit at the price of the tier the total reaches. The same
      1,500 cost {symbol}750.
    </span>
    <span className="block">
      <strong>Package</strong>: a price per block of units, a block started counting in full.
    </span>
    <span className="block">
      A tier&apos;s flat fee is added once usage enters it; its cap is the most the tier costs.
    </span>
  </span>
)

export const ChargeBody = ({
  shape,
  onShapeChange,
  methods,
  currency,
  term,
  lock,
  context,
  newProduct,
  familyLocalId,
  manyLevels = false,
  name,
  lead,
  footer,
}: {
  shape: ChargeShape
  onShapeChange: (shape: ChargeShape) => void
  methods: Methods<z.ZodType>
  currency: string
  term: BillingPeriod
  lock?: ChargeLock
  context: ChargeBodyContext
  newProduct: boolean
  /** The product family a new usage product's metric is picked from. */
  familyLocalId?: string
  /** A capacity charge keeps several levels (a plan's); otherwise the one level priced. */
  manyLevels?: boolean
  /** The charge's name, its invoice line's. */
  name?: string
  /** Above the fields: what it charges for, notes. */
  lead?: ReactNode
  footer: { remove?: { label: string; onClick: () => void; disabled?: boolean }; actions: ReactNode }
}) => {
  const control = methods.control as unknown as Control<FieldValues>
  const values = (useWatch({ control }) ?? {}) as ChargeValues
  const s = normalizeShape(shape)
  const [asTiers, setAsTiers] = useState(false)
  const [more, setMore] = useState(false)
  const creditUnit = useSelectedCreditUnit(control)

  // The charge's metric is picked here, unless its shape is fixed.
  const metricsQuery = useQuery(
    listBillableMetrics,
    newProduct && s.source === 'metered' ? (familyLocalId ? { familyLocalId } : {}) : skipToken
  )
  const metricId = (values.metricId as string | undefined) || context.metricId
  const metric =
    metricsQuery.data?.billableMetrics?.find(m => m.id === metricId) ??
    (context.metric?.id === metricId ? context.metric : undefined)
  const unit: MetricUnit | undefined = metricUnitFromProto(metric)
  // Overage on a metric counting money reads as a share of it, unless priced in a credit unit.
  const moneyUnit = unitIsCurrency(unit, currency) && !values.creditUnitId
  const seat = singular((values.slotUnitName as string | undefined) || context.slotUnitName || 'Seats')
  // How often it bills: the form's own, else the cadence it opened on.
  const cadence = values.term ? termFromFormValue(values.term as Cadence) : term
  const per = termPer(cadence)
  const recurring = s.recurrence === 'recurring'
  const allowance = s.model === 'tiered' && s.source === 'metered' && isAllowanceTiers(values.rows) && !asTiers

  const applyShape = (next: ChargeShape) => {
    const normalized = normalizeShape(next)
    methods.reset(valuesForShape(normalized, methods.getValues() as ChargeValues, shape))
    onShapeChange(normalized)
  }
  const fits = (change: Partial<ChargeShape>) => {
    const next = bestFit(shape, change, lock)
    return next && Object.entries(change).every(([k, v]) => next[k as keyof ChargeShape] === v)
      ? next
      : undefined
  }
  // Usage per unit with some included: a free first tier, or none (plain usage per unit).
  const setIncluded = (raw: string) => {
    const next = withIncludedUsage(s, methods.getValues() as ChargeValues, raw)
    if (!shapeFits(next.shape, lock)) return
    methods.reset(next.values)
    if (next.shape.model !== s.model) onShapeChange(next.shape)
  }
  // "Add a tier" below a price per unit: its tiers, one more after them.
  const addTier = () => {
    const current = methods.getValues() as ChargeValues
    if (allowance) {
      methods.setValue('rows', splitLastTier(current.rows as TierRowLike[]), { shouldDirty: true })
    } else {
      const next = fits({ model: 'tiered' })
      if (!next) return
      const first = { firstUnit: BigInt(0), unitPrice: String(current.unitPrice ?? '0') }
      methods.reset({ ...valuesForShape(next, current, shape), rows: splitLastTier([first]) })
      onShapeChange(next)
    }
    setAsTiers(true)
  }
  const pickModel = (model: RatingModel) => {
    // Graduated tiers that are an allowance read back as a price per unit, some included.
    if (model === 'per_unit' && s.model === 'tiered' && isAllowanceTiers(values.rows)) {
      return setAsTiers(false)
    }
    const next = fits({ model })
    if (next) applyShape(next)
    setAsTiers(model === 'tiered')
  }
  const setCadence = (next: Cadence) => methods.setValue('term', next, { shouldDirty: true })
  const models = MODEL_OPTIONS[s.source].filter(
    ([model]) => model === s.model || !!fits({ model })
  )
  const billings = billingsFor(s).filter(billing => billing === s.billing || fits({ billing }))
  const pricedBy = s.model !== 'credit_pack' && s.model !== 'included' && models.length > 1
  const charged = s.source === 'fixed' && s.model !== 'credit_pack'
  const hasMore = (recurring && billings.length > 1) || s.source === 'seats'
  // A single amount carries its cadence ("/ month ▾"); tiers, usage and levels say it below.
  const cadenceInField =
    (s.source === 'fixed' && ['flat', 'per_unit', 'package'].includes(s.model)) ||
    (s.source === 'seats' && ['per_unit', 'package'].includes(s.model))
  const every = recurring ? <CadenceMenu value={cadence} onChange={setCadence} /> : 'once'
  // Plain usage and usage with some included are one view: typing an amount keeps its field.
  const view = s.source === 'metered' && (s.model === 'per_unit' || allowance) ? 'unit' : s.model

  return (
    <ChargeLayout
      preview={
        <InvoicePreview
          key={`${s.source}-${s.model === 'included'}`}
          shape={s}
          values={values}
          name={name}
          currency={currency}
          per={per}
          adverb={termAdverb(cadence)}
          seat={seat}
          unit={unit}
          metricName={metric?.name}
          creditUnitName={creditUnit?.pluralName || creditUnit?.name}
        />
      }
      footer={
        <FormFooter remove={footer.remove}>{footer.actions}</FormFooter>
      }
    >
      {lead}
      {newProduct && s.source === 'metered' && (
        <div className="max-w-sm">
          <MetricCombobox methods={methods} familyLocalId={familyLocalId} />
        </div>
      )}
      {newProduct && s.source === 'seats' && (
        <QuantityField
          control={control}
          setValue={methods.setValue as unknown as UseFormSetValue<FieldValues>}
          quantityId={values.quantityId as string | undefined}
        />
      )}
      {charged && (
        <FieldRow label="Charged">
          <Segmented<Recurrence>
            label="Charged"
            value={s.recurrence}
            options={[
              { value: 'recurring', label: 'Every period', disabled: !fits({ recurrence: 'recurring' }) },
              { value: 'one_time', label: 'Once', disabled: !fits({ recurrence: 'one_time' }) },
            ]}
            onChange={recurrence => {
              const next = fits({ recurrence })
              if (next) applyShape(next)
            }}
          />
        </FieldRow>
      )}
      {pricedBy && (
        <FieldRow label={<Help label="Priced" text={pricedHelp(currencySymbol(currency))} wide />}>
          <Segmented<RatingModel>
            label="Priced"
            value={allowance ? 'per_unit' : s.model}
            options={models.map(([model, label]) => ({ value: model, label }))}
            onChange={pickModel}
          />
        </FieldRow>
      )}

      <div key={`${s.source}-${view}`} className="space-y-4">
        <Fields
          shape={s}
          control={control}
          currency={currency}
          per={per}
          every={every}
          seat={seat}
          unit={unit}
          metricId={metricId}
          allowance={allowance}
          included={includedUsage(s, values)}
          onIncluded={setIncluded}
          onAddTier={addTier}
          levels={{ many: manyLevels, moneyUnit }}
        />
      </div>
      {recurring && !cadenceInField && (
        <p className="flex items-center gap-1 text-xs text-muted-foreground">
          Billed
          <CadenceMenu value={cadence} onChange={setCadence} format={p => `every ${p}`} />
        </p>
      )}

      {hasMore && (
        <Collapsible open={more} onOpenChange={setMore} className="space-y-3">
          <CollapsibleTrigger asChild>
            <button
              type="button"
              className="inline-flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground"
            >
              <ChevronRightIcon size={12} className={cn('transition-transform', more && 'rotate-90')} />
              More pricing options
            </button>
          </CollapsibleTrigger>
          <CollapsibleContent className="space-y-4 rounded-md border border-border bg-muted/20 p-3">
            {recurring && billings.length > 1 && (
              <OptionRow label="Billed">
                <Segmented<Billing>
                  label="Billed"
                  value={s.billing}
                  options={billings.map(billing => ({
                    value: billing,
                    label: billing === 'advance' ? 'In advance' : 'In arrears, after the period',
                  }))}
                  onChange={billing => {
                    const next = fits({ billing })
                    if (next) applyShape(next)
                  }}
                />
              </OptionRow>
            )}
            {s.source === 'seats' && (
              <OptionRow label="Limits">
                <div className="grid max-w-sm grid-cols-2 gap-3">
                  <UnitField control={control} name="minSlots" label="At least" kind="count" suffix={`${seat}s`} />
                  <UnitField control={control} name="maxSlots" label="At most" kind="count" suffix={`${seat}s`} />
                </div>
              </OptionRow>
            )}
          </CollapsibleContent>
        </Collapsible>
      )}
    </ChargeLayout>
  )
}

const OptionRow = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="grid gap-2 sm:grid-cols-[88px_1fr] sm:items-start">
    <span className="pt-1 text-xs text-muted-foreground">{label}</span>
    <div>{children}</div>
  </div>
)

/** The usage included each period, none by default: a free first tier when set. */
const IncludedField = ({
  value,
  per,
  units,
  onChange,
}: {
  value: bigint
  per: string
  units: string
  onChange: (raw: string) => void
}) => {
  const id = useId()
  return (
    <div className="space-y-1.5">
      <label htmlFor={id} className="block text-xs text-muted-foreground">
        <Help
          label={`Included each ${per}`}
          text={`Usage up to this amount each ${per} is free; the price applies above it. None: every unit is billed.`}
        />
      </label>
      <div className="flex h-9 items-center gap-1.5 rounded-md border border-input bg-background px-2.5 text-sm shadow-sm focus-within:ring-1 focus-within:ring-ring">
        <input
          id={id}
          inputMode="numeric"
          placeholder="None"
          className="min-w-0 flex-1 bg-transparent font-mono tabular-nums outline-none placeholder:font-sans placeholder:text-muted-foreground/60"
          value={value > BigInt(0) ? grouped(value.toString()) : ''}
          onChange={e => onChange(digitsOnly(e.target.value))}
        />
        <span className="shrink-0 text-xs text-muted-foreground">{units}</span>
      </div>
    </div>
  )
}

const Fields = ({
  shape,
  control,
  currency,
  per,
  every,
  seat,
  unit,
  metricId,
  allowance,
  included,
  onIncluded,
  onAddTier,
  levels,
}: {
  shape: ChargeShape
  control: Control<FieldValues>
  currency: string
  per: string
  /** After an amount: its cadence menu, or "once". */
  every: ReactNode
  seat: string
  unit?: MetricUnit
  metricId?: string
  allowance: boolean
  /** Usage included each period, and its setter (usage per unit only). */
  included: bigint
  onIncluded: (raw: string) => void
  /** Turns a price per unit into tiers, one added. */
  onAddTier: () => void
  levels: { many: boolean; moneyUnit: boolean }
}) => {
  const symbol = currencySymbol(currency)
  const perUnit = formatPerUnit(unit)
  const units = pluralOfUnit(unit)
  // What a priced unit is: "token", or "1M tokens" when it is itself many.
  const priced = perUnit.replace(/^per /, '')
  const grid = 'grid gap-3 sm:grid-cols-2'
  const quantity = (
    <UnitField control={control} name="quantity" label="Quantity" kind="count" suffix="units" />
  )
  const tiers = (unitSingular: string) => (
    <TierEditor
      control={control}
      currency={currency}
      mode={shape.model === 'volume' ? 'volume' : 'graduated'}
      unitSingular={unitSingular}
    />
  )
  const pack = (unitPlural: string) => (
    <div className={grid}>
      <UnitField control={control} name="packagePrice" label="Price per package" prefix={symbol} suffix={every} />
      <UnitField control={control} name="blockSize" label="Package size" kind="count" suffix={unitPlural} />
    </div>
  )

  switch (shape.source) {
    case 'fixed':
      switch (shape.model) {
        case 'flat':
          return (
            <div className={grid}>
              <UnitField control={control} name="rate" label="Amount" prefix={symbol} suffix={every} />
            </div>
          )
        case 'per_unit':
          return (
            <div className={grid}>
              {quantity}
              <UnitField control={control} name="unitPrice" label="Price per unit" prefix={symbol} suffix={every} />
            </div>
          )
        case 'tiered':
        case 'volume':
          return (
            <>
              <div className={grid}>{quantity}</div>
              {tiers('unit')}
            </>
          )
        case 'package':
          return (
            <>
              <div className={grid}>{quantity}</div>
              {pack('units')}
            </>
          )
        default:
          return <ChargePriceFields shape={shape} control={control} currency={currency} />
      }
    case 'seats':
      switch (shape.model) {
        case 'per_unit':
          return (
            <div className={grid}>
              <UnitField
                control={control}
                name="unitPrice"
                label={`Price per ${seat}`}
                prefix={symbol}
                suffix={
                  <span className="inline-flex items-center gap-0.5">
                    per {seat} {every}
                  </span>
                }
              />
            </div>
          )
        case 'tiered':
        case 'volume':
          return tiers(seat)
        case 'package':
          return pack(`${seat}s`)
        default:
          return null
      }
    case 'metered':
      if (shape.model === 'per_unit' || allowance) {
        return (
          <div className="space-y-2">
            <div className={grid}>
              <UnitField
                key={allowance ? 'above' : 'each'}
                control={control}
                name={allowance ? 'rows.1.unitPrice' : 'unitPrice'}
                label={allowance ? 'Price above it' : 'Price'}
                prefix={symbol}
                suffix={perUnit}
              />
              <IncludedField
                value={included}
                per={per}
                units={/^\d/.test(priced) ? `× ${priced}` : units}
                onChange={onIncluded}
              />
            </div>
            <button
              type="button"
              onClick={onAddTier}
              className="inline-flex items-center gap-1 text-xs font-medium text-brand hover:underline focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <PlusIcon size={12} /> Add a tier
            </button>
          </div>
        )
      }
      switch (shape.model) {
        case 'included':
          return (
            <LevelEditor
              control={control}
              currency={currency}
              per={per}
              unitPlural={units}
              perUnit={perUnit}
              many={levels.many}
              moneyUnit={levels.moneyUnit}
            />
          )
        case 'tiered':
        case 'volume':
          return tiers(priced)
        case 'package':
          return pack(units)
        default:
          return (
            <ChargePriceFields shape={shape} control={control} currency={currency} metricId={metricId} />
          )
      }
  }
}
