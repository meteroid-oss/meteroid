import { skipToken } from '@connectrpc/connect-query'
import {
  Button,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  GenericFormField,
  Input,
  InputFormField,
  Label,
  RadioGroup,
  RadioGroupItem,
  SelectFormField,
  SelectItem,
  SwitchFormField,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { HelpCircleIcon, PlusIcon, XIcon } from 'lucide-react'
import { memo, useCallback, useEffect, useMemo, useState } from 'react'
import {
  Control,
  FieldValues,
  useController,
  useFieldArray,
  useFormContext,
  useFormState,
  useWatch,
} from 'react-hook-form'

import { Gate } from '@/components/Gate'
import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { SimpleTable } from '@/components/table/SimpleTable'
import { NewRateCardDialog } from '@/features/productCatalog/ratecards/NewRateCardDialog'
import { RateCardSheet } from '@/features/productCatalog/ratecards/RateCardSheet'
import { cardRatesForPrice } from '@/features/productCatalog/ratecards/bridge'
import {
  UNIT_SIZES,
  dimensionName,
  listNames,
  rateCardSummary,
  unitSizeLabel,
} from '@/features/productCatalog/ratecards/rateTable'
import { useMatrixDimensions } from '@/hooks/useMatrixDimensions'
import { useQuery } from '@/lib/connectrpc'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  getRateCard,
  listRateCards,
} from '@/rpc/api/ratecards/v1/ratecards-RateCardsService_connectquery'

import { CreditPackUnitField, useGrantedCreditUnit } from './CreditPackUnitField'
import { useCreditUnit } from './CreditUnitPriceFields'
import { creditValue, packCreditPrice } from './creditValue'
import { PricingType, isZeroAmount, type DimensionRateRow, type TierRowSchema } from './schemas'

import type { PriceRates } from '@/features/productCatalog/ratecards/bridge'
import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import type { RateCard } from '@/rpc/api/ratecards/v1/models_pb'
import type { z } from 'zod'

type TierRow = z.infer<typeof TierRowSchema>

export interface MatrixDimension {
  key: string
  value: string
}

export interface DimensionCombination {
  dimension1: MatrixDimension
  dimension2?: MatrixDimension
}

interface PricingFieldsProps {
  pricingType: PricingType
  control: Control<FieldValues>
  currency: string
  /** For capacity pricing: what the overage is priced in, when not `currency` (a credit unit) */
  overageCurrency?: string
  /** For rate card pricing, only the cards of this metric are offered; for rates by dimension,
   * its declared dimension values pre-fill the rows */
  metricId?: string
}

/**
 * Reusable pricing input fields that render based on the pricing type.
 * Designed to be embedded inside any react-hook-form Form.
 * The parent form's schema must match the expected schema for the given pricing type.
 */
export function PricingFields({
  pricingType,
  control,
  currency,
  overageCurrency,
  metricId,
}: PricingFieldsProps) {
  switch (pricingType) {
    case 'capacity':
      return (
        <CapacityPricingFields
          control={control}
          currency={currency}
          overageCurrency={overageCurrency}
        />
      )
    case 'perUnit':
      return <PerUnitPricingFields control={control} currency={currency} />
    case 'tiered':
    case 'volume':
      return <TierTableFields control={control} currency={currency} />
    case 'package':
      return <PackagePricingFields control={control} currency={currency} />
    case 'rateCard':
      return <RateCardPricingFields control={control} metricId={metricId} />
    case 'dimensionRates':
      return <DimensionRatesFields control={control} currency={currency} metricId={metricId} />
    case 'credit':
      return <CreditPricingFields control={control} currency={currency} />
    // A flat, seat or fixed-count price is laid out by the charge form itself.
    default:
      return null
  }
}

// --- Rate card ---

const SHARED_CARD =
  'A rate card is kept outside the plan: every price rated by it bills its rates, so editing them changes them for all of those prices.'

/** A (?) beside a field's label: what it means, on hover. */
const LabelHint = ({ text }: { text: string }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <HelpCircleIcon size={12} aria-label={text} className="cursor-help text-muted-foreground" />
    </TooltipTrigger>
    <TooltipContent className="max-w-64 text-xs">{text}</TooltipContent>
  </Tooltip>
)

/**
 * The shared rate card a price is rated by, and its markup. With `onCopy`, the card's rates (in
 * `currency`, the markup applied) can be copied into the price, which then holds its own.
 */
export function RateCardPricingFields({
  control,
  metricId,
  currency,
  onCopy,
}: {
  control: Control<FieldValues>
  metricId?: string
  currency?: string
  onCopy?: (rates: PriceRates) => void
}) {
  const query = useQuery(listRateCards, metricId ? { billableMetricId: metricId } : skipToken)
  const metricCards = useMemo(
    () => (query.data?.cards ?? []).filter(card => card.metricId === metricId),
    [query.data, metricId]
  )
  const { field } = useController({ control, name: 'rateCardId' })
  const rateCardId = field.value as string | undefined
  const creditUnitId = (useWatch({ control, name: 'creditUnitId' }) as string | undefined) || undefined
  const [creating, setCreating] = useState(false)
  const [openId, setOpenId] = useState<string | null>(null)

  // A card of the previous metric can't rate this one.
  const { onChange } = field
  // Not while the list refetches: a card just made is in the next one.
  useEffect(() => {
    if (!rateCardId || !query.data || query.isFetching) return
    if (!metricId || !metricCards.some(card => card.id === rateCardId)) onChange('')
  }, [rateCardId, metricId, metricCards, query.data, query.isFetching, onChange])

  // A money card rates a price in any currency; a credit card, a price in its credit unit.
  const fits = (card: RateCard) => (card.creditUnitId ?? '') === (creditUnitId ?? '')
  const cards = metricCards.filter(card => fits(card) || card.id === rateCardId)
  const selected = metricCards.find(card => card.id === rateCardId)
  const selectedRates = useQuery(getRateCard, selected ? { id: selected.id } : skipToken).data
    ?.rates
  const markup = useWatch({ control, name: 'markup' }) as string | undefined
  const copy =
    onCopy && currency && selected && selectedRates && fits(selected)
      ? cardRatesForPrice(selected, selectedRates, currency, markup)
      : undefined
  const unit = useQuery(getBillableMetric, metricId ? { id: metricId } : skipToken).data
    ?.billableMetric?.unitLabel
  const selectedCredit = useCreditUnit(selected?.creditUnitId)

  if (!metricId) {
    return <p className="py-2 text-sm text-muted-foreground">Choose the metric first.</p>
  }
  const newCard = (
    <Gate perm="catalog:manage">
      <Button type="button" variant="outline" size="sm" hasIcon onClick={() => setCreating(true)}>
        <PlusIcon className="h-4 w-4" />
        New rate card
      </Button>
    </Gate>
  )
  return (
    <div className="space-y-4">
      {query.data && !cards.length ? (
        <div className="flex items-center justify-between gap-4 py-2">
          <p className="text-sm text-muted-foreground">
            {metricCards.length
              ? creditUnitId
                ? 'No rate card of this metric rates in the price\'s credit unit.'
                : 'No rate card of this metric rates in money.'
              : 'No rate card for this metric yet.'}
          </p>
          {newCard}
        </div>
      ) : (
        <div className="space-y-1">
          <div className="flex items-end gap-2">
            <SelectFormField
              control={control}
              name="rateCardId"
              label="Shared rate card"
              labelTooltip={<LabelHint text={SHARED_CARD} />}
              placeholder={query.isLoading ? 'Loading...' : 'Choose'}
              containerClassName="flex-1"
            >
              {cards.map(card => (
                <SelectItem key={card.id} value={card.id}>
                  {card.name} · by {listNames(card.dimensions.map(dimensionName))}
                </SelectItem>
              ))}
            </SelectFormField>
            {newCard}
          </div>
          {selected && (
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
              {selectedRates && (
                <span className="text-muted-foreground">
                  {rateCardSummary(
                    selected,
                    selectedRates,
                    unit,
                    selectedCredit?.pluralName || selectedCredit?.name
                  )}
                </span>
              )}
              <button
                type="button"
                onClick={() => setOpenId(selected.id)}
                className="font-medium text-foreground hover:underline"
              >
                Edit for every plan using it…
              </button>
              {onCopy && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    {/* A disabled button gets no pointer events: its wrapper shows why. */}
                    <span>
                      <button
                        type="button"
                        disabled={!copy}
                        onClick={() => copy && onCopy(copy)}
                        className="font-medium text-foreground hover:underline disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:no-underline"
                      >
                        Copy into this plan
                      </button>
                    </span>
                  </TooltipTrigger>
                  <TooltipContent className="max-w-64 text-xs">
                    {copy
                      ? "The card's rates today, the markup applied, become this price's own: later changes to the card no longer reach it."
                      : `The card has no rate in ${currency ?? 'this currency'} to copy.`}
                  </TooltipContent>
                </Tooltip>
              )}
            </div>
          )}
          {selected && !fits(selected) && (
            <p className="text-xs text-destructive">
              {selected.creditUnitId
                ? 'This card rates in a credit unit, not in the price\'s.'
                : 'This card rates in money, the price in credits.'}
            </p>
          )}
        </div>
      )}
      <InputFormField
        control={control}
        name="markup"
        label="Markup"
        rightText="%"
        placeholder="0"
        labelTooltip={
          <LabelHint text="On every rate of the card: -20 for 20% off. Empty: the card's rates as they are." />
        }
        containerClassName="max-w-xs"
      />
      {creating && (
        <NewRateCardDialog
          metricId={metricId}
          creditUnitId={creditUnitId}
          onClose={() => setCreating(false)}
          onCreated={card => {
            setCreating(false)
            onChange(card.id)
            setOpenId(card.id)
          }}
        />
      )}
      {openId && <RateCardSheet id={openId} onClose={() => setOpenId(null)} />}
    </div>
  )
}

// --- Capacity (threshold table) ---

function CapacityPricingFields({
  control,
  currency,
  overageCurrency,
}: {
  control: Control<FieldValues>
  currency: string
  overageCurrency?: string
}) {
  const { fields, append, remove } = useFieldArray({
    control,
    name: 'thresholds',
  })

  const addThreshold = useCallback(() => {
    append({ included: 0, rate: '0.00', overageRate: '0.00000000' })
  }, [append])

  useEffect(() => {
    if (fields.length === 0) {
      append({ included: 0, rate: '0.00', overageRate: '0.00000000' })
    }
  }, [fields.length, append])

  const columns = useMemo<ColumnDef<{ included: number; rate: string; overageRate: string }>[]>(
    () => [
      {
        header: 'Included',
        cell: ({ row }) => (
          <FormField
            control={control}
            name={`thresholds.${row.index}.included`}
            render={({ field }) => (
              <FormItem>
                <FormControl>
                  <Input
                    {...field}
                    type="number"
                    min={0}
                    step={1}
                    onChange={e => field.onChange(Number(e.target.value))}
                    value={field.value ?? 0}
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        ),
      },
      {
        header: 'Rate',
        cell: ({ row }) => (
          <GenericFormField
            control={control}
            name={`thresholds.${row.index}.rate`}
            render={({ field }) => (
              <UncontrolledPriceInput {...field} currency={currency} showCurrency={false} />
            )}
          />
        ),
      },
      {
        header: overageCurrency ? 'Overage rate (credits)' : 'Overage rate',
        cell: ({ row }) => (
          <GenericFormField
            control={control}
            name={`thresholds.${row.index}.overageRate`}
            render={({ field }) => (
              <UncontrolledPriceInput
                {...field}
                currency={overageCurrency ?? currency}
                unitLabel={overageCurrency ? 'credits' : undefined}
                showCurrency={false}
                precision={8}
              />
            )}
          />
        ),
      },
      {
        header: '',
        id: 'remove',
        cell: ({ row }) =>
          fields.length <= 1 ? null : (
            <Button variant="link" size="icon" onClick={() => remove(row.index)}>
              <XIcon size={12} />
            </Button>
          ),
      },
    ],
    [control, currency, overageCurrency, fields.length, remove]
  )

  return (
    <>
      <SimpleTable
        columns={columns}
        data={fields as unknown as { included: number; rate: string; overageRate: string }[]}
      />
      <Button variant="link" onClick={addThreshold}>
        + Add threshold
      </Button>
    </>
  )
}

// --- Per Unit ---

function PerUnitPricingFields({
  control,
  currency,
}: {
  control: Control<FieldValues>
  currency: string
}) {
  return (
    <GenericFormField
      control={control}
      name="unitPrice"
      label="Price per unit"
      render={({ field }) => (
        <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" precision={8} />
      )}
    />
  )
}

// --- Tiered / Volume ---

function TierTableFields({
  control,
  currency,
}: {
  control: Control<FieldValues>
  currency: string
}) {
  const { fields, append, remove } = useFieldArray({
    control,
    name: 'rows',
  })
  const { setValue } = useFormContext()

  const [showFlatFee, setShowFlatFee] = useState(false)
  const [showFlatCap, setShowFlatCap] = useState(false)

  // Auto-detect: if any row already has flatFee/flatCap, show those columns
  useEffect(() => {
    const rows = fields as unknown as TierRow[]
    if (rows.some(r => r.flatFee != null && r.flatFee !== '')) setShowFlatFee(true)
    if (rows.some(r => r.flatCap != null && r.flatCap !== '')) setShowFlatCap(true)
    // Only check on initial mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const toggleFlatFee = useCallback(
    (checked: boolean) => {
      setShowFlatFee(checked)
      if (!checked) {
        fields.forEach((_, i) => setValue(`rows.${i}.flatFee`, undefined))
      }
    },
    [fields, setValue]
  )

  const toggleFlatCap = useCallback(
    (checked: boolean) => {
      setShowFlatCap(checked)
      if (!checked) {
        fields.forEach((_, i) => setValue(`rows.${i}.flatCap`, undefined))
      }
    },
    [fields, setValue]
  )

  const addTier = useCallback(() => {
    const lastTier = fields[fields.length - 1] as unknown as TierRow | undefined
    const firstUnit = lastTier ? BigInt(lastTier.firstUnit) + BigInt(1) : BigInt(0)
    append({ firstUnit, unitPrice: '' })
  }, [fields, append])

  const columns = useMemo<ColumnDef<TierRow>[]>(
    () => [
      {
        header: 'First unit',
        cell: ({ row }) => <FirstUnitField control={control} rowIndex={row.index} />,
      },
      {
        header: 'Last unit',
        cell: ({ row }) => <LastUnitCell control={control} rowIndex={row.index} />,
      },
      {
        header: 'Per unit',
        cell: ({ row }) => (
          <GenericFormField
            control={control}
            name={`rows.${row.index}.unitPrice`}
            render={({ field }) => (
              <UncontrolledPriceInput
                {...field}
                currency={currency}
                showCurrency={false}
                className="max-w-xs"
                precision={8}
              />
            )}
          />
        ),
      },
      ...(showFlatFee
        ? [
            {
              header: 'Flat fee',
              cell: ({ row }: { row: { index: number } }) => (
                <GenericFormField
                  control={control}
                  name={`rows.${row.index}.flatFee`}
                  render={({ field }) => (
                    <UncontrolledPriceInput
                      {...field}
                      currency={currency}
                      showCurrency={false}
                      className="max-w-xs"
                      precision={2}
                    />
                  )}
                />
              ),
            } as ColumnDef<TierRow>,
          ]
        : []),
      ...(showFlatCap
        ? [
            {
              header: 'Flat cap',
              cell: ({ row }: { row: { index: number } }) => (
                <GenericFormField
                  control={control}
                  name={`rows.${row.index}.flatCap`}
                  render={({ field }) => (
                    <UncontrolledPriceInput
                      {...field}
                      currency={currency}
                      showCurrency={false}
                      className="max-w-xs"
                      precision={2}
                    />
                  )}
                />
              ),
            } as ColumnDef<TierRow>,
          ]
        : []),
      {
        header: () => (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon" className="h-6 w-6">
                <PlusIcon size={14} />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuCheckboxItem checked={showFlatFee} onCheckedChange={toggleFlatFee}>
                Flat fee
              </DropdownMenuCheckboxItem>
              <DropdownMenuCheckboxItem checked={showFlatCap} onCheckedChange={toggleFlatCap}>
                Flat cap
              </DropdownMenuCheckboxItem>
            </DropdownMenuContent>
          </DropdownMenu>
        ),
        id: 'actions',
        cell: ({ row }) =>
          fields.length <= 2 || row.index === 0 ? null : (
            <Button
              variant="link"
              size="icon"
              onClick={() => remove(row.index)}
              disabled={fields.length <= 2}
            >
              <XIcon size={12} />
            </Button>
          ),
      },
    ],
    [
      control,
      currency,
      fields.length,
      remove,
      showFlatFee,
      showFlatCap,
      toggleFlatFee,
      toggleFlatCap,
    ]
  )

  return (
    <>
      <SimpleTable columns={columns} data={fields as unknown as TierRow[]} />
      <Button variant="link" onClick={addTier}>
        + Add tier
      </Button>
    </>
  )
}

const FirstUnitField = memo(
  ({ control, rowIndex }: { control: Control<FieldValues>; rowIndex: number }) => {
    const prevRowValue = useWatch({
      control,
      name: `rows.${rowIndex - 1}`,
    })

    const isFirst = rowIndex === 0

    return (
      <FormField
        control={control}
        name={`rows.${rowIndex}.firstUnit`}
        rules={{
          min: isFirst ? 0 : Number(prevRowValue?.firstUnit ?? 0) + 1,
        }}
        render={({ field }) => (
          <FormItem>
            <FormControl>
              <Input
                {...field}
                type="number"
                min={isFirst ? 0 : Number(prevRowValue?.firstUnit ?? 0) + 1}
                onChange={e => field.onChange(BigInt(e.target.value))}
                value={Number(field.value)}
                disabled={isFirst}
                placeholder={isFirst ? '0' : ''}
              />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />
    )
  }
)

const LastUnitCell = ({
  control,
  rowIndex,
}: {
  control: Control<FieldValues>
  rowIndex: number
}) => {
  const nextRow = useWatch({
    control,
    name: `rows.${rowIndex + 1}`,
  })

  return !nextRow ? <>&#8734;</> : <>{`${BigInt(nextRow.firstUnit)}`}</>
}

// --- Package ---

function PackagePricingFields({
  control,
  currency,
}: {
  control: Control<FieldValues>
  currency: string
}) {
  return (
    <div className="space-y-4">
      <InputFormField
        name="blockSize"
        label="Block size"
        type="number"
        step={1}
        className="max-w-xs"
        control={control}
      />
      <GenericFormField
        control={control}
        name="packagePrice"
        label="Price per block"
        render={({ field }) => (
          <UncontrolledPriceInput
            {...field}
            currency={currency}
            className="max-w-xs"
            precision={8}
          />
        )}
      />
    </div>
  )
}

// --- Rates by dimension, held by the price ---

const MAX_DIMENSIONS = 4

/**
 * A rate per key of the usage's dimensions, a column per dimension: a value, a prefix ending in
 * `*`, or `*` (empty) for any. The metric's declared dimension values pre-fill a new table.
 */
function DimensionRatesFields({
  control,
  currency,
  metricId,
}: {
  control: Control<FieldValues>
  currency: string
  metricId?: string
}) {
  const { getValues, setValue } = useFormContext()
  const { fields, append, remove, replace } = useFieldArray({ control, name: 'keyRates' })
  const dimensions = (useWatch({ control, name: 'dimensions' }) as string[] | undefined) ?? []
  const unitSize = useWatch({ control, name: 'unitSize' }) as string | undefined
  const onUnmatched = useWatch({ control, name: 'onUnmatched' }) as string | undefined
  const metric = useQuery(getBillableMetric, metricId ? { id: metricId } : skipToken).data
    ?.billableMetric
  const { dimensionHeaders, validCombinations } = useMatrixDimensions(metric)
  const [more, setMore] = useState(
    () => (unitSize ?? '1') !== '1' || onUnmatched === 'ignore'
  )
  const { errors } = useFormState({ control, name: 'keyRates' })
  const keyRatesError = (errors.keyRates ?? {}) as {
    message?: string
    root?: { message?: string }
  }

  // A new table starts from the metric's dimensions and the values it declares, else one
  // dimension to name.
  useEffect(() => {
    const named = ((getValues('dimensions') ?? []) as string[]).some(d => d.trim())
    if (named || (getValues('keyRates') ?? []).length) return
    if (metricId && !metric) return
    if (dimensionHeaders?.length) {
      setValue('dimensions', dimensionHeaders.slice(0, MAX_DIMENSIONS))
      replace(
        (validCombinations ?? []).map(combo => ({
          key: [combo.dimension1.value, combo.dimension2?.value ?? '*'].slice(
            0,
            dimensionHeaders.length
          ),
          rate: '',
        }))
      )
    } else if (!(getValues('dimensions') ?? []).length) {
      setValue('dimensions', [''])
    }
  }, [metricId, metric, dimensionHeaders, validCombinations, getValues, setValue, replace])

  // Each row keeps a pattern per dimension.
  const resize = (next: string[], at?: number) => {
    const rows = (getValues('keyRates') ?? []) as DimensionRateRow[]
    setValue('dimensions', next, { shouldDirty: true })
    replace(
      rows.map(row => ({
        ...row,
        key:
          at === undefined ? [...row.key, '*'] : row.key.filter((_, index) => index !== at),
      }))
    )
  }

  return (
    <div className="space-y-2">
      <div className="overflow-x-auto rounded-md border border-border">
        <table className="w-full text-sm">
          <thead className="bg-muted/40">
            <tr>
              {dimensions.map((_, d) => (
                <th key={d} className="px-2 py-1.5 text-left font-normal">
                  <div className="flex items-center gap-1">
                    <FormField
                      control={control}
                      name={`dimensions.${d}`}
                      render={({ field }) => (
                        <Input
                          {...field}
                          value={field.value ?? ''}
                          aria-label={`Dimension ${d + 1}`}
                          placeholder="Event property, e.g. model"
                          className="h-7 font-mono text-xs"
                        />
                      )}
                    />
                    {dimensions.length > 1 && (
                      <button
                        type="button"
                        aria-label="Remove the dimension"
                        onClick={() => resize(dimensions.filter((_, i) => i !== d), d)}
                        className="text-muted-foreground hover:text-foreground"
                      >
                        <XIcon size={12} />
                      </button>
                    )}
                  </div>
                </th>
              ))}
              <th className="w-40 px-2 py-1.5 text-left text-xs font-normal text-muted-foreground">
                Rate {unitSizeLabel(unitSize ?? '1')}
              </th>
              <th className="w-8" />
            </tr>
          </thead>
          <tbody>
            {fields.map((row, i) => (
              <tr key={row.id} className="border-t border-border">
                {dimensions.map((_, d) => (
                  <td key={d} className="px-2 py-1">
                    <FormField
                      control={control}
                      name={`keyRates.${i}.key.${d}`}
                      render={({ field }) => (
                        <Input
                          {...field}
                          value={field.value === '*' ? '' : (field.value ?? '')}
                          onChange={e => field.onChange(e.target.value || '*')}
                          aria-label={`${dimensionName(dimensions[d] || `Dimension ${d + 1}`)} of row ${i + 1}`}
                          placeholder="Any"
                          className="h-8 font-mono text-xs"
                        />
                      )}
                    />
                  </td>
                ))}
                <td className="px-2 py-1">
                  <GenericFormField
                    control={control}
                    name={`keyRates.${i}.rate`}
                    render={({ field }) => (
                      <UncontrolledPriceInput {...field} currency={currency} precision={8} />
                    )}
                  />
                </td>
                <td className="px-1">
                  <button
                    type="button"
                    aria-label={`Remove row ${i + 1}`}
                    onClick={() => remove(i)}
                    className="text-muted-foreground hover:text-foreground"
                  >
                    <XIcon size={14} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex flex-wrap items-center gap-3">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          hasIcon
          onClick={() => append({ key: dimensions.map(() => '*'), rate: '' })}
        >
          <PlusIcon className="h-4 w-4" />
          Add a rate
        </Button>
        {dimensions.length < MAX_DIMENSIONS && (
          <Button type="button" variant="ghost" size="sm" hasIcon onClick={() => resize([...dimensions, ''])}>
            <PlusIcon className="h-4 w-4" />
            Add a dimension
          </Button>
        )}
      </div>
      {(keyRatesError.root?.message ?? keyRatesError.message) && (
        <p className="text-xs text-destructive">
          {keyRatesError.root?.message ?? keyRatesError.message}
        </p>
      )}
      <p className="text-xs text-muted-foreground">
        An empty cell matches any value, <code>gpt-*</code> any value starting with gpt-. Usage
        takes the rate of its most specific row.
      </p>
      <button
        type="button"
        onClick={() => setMore(!more)}
        className="text-xs font-medium text-muted-foreground hover:text-foreground"
      >
        {more ? 'Fewer options' : 'More options'}
      </button>
      {more && (
        <div className="grid max-w-md gap-3 sm:grid-cols-2">
          <SelectFormField control={control} name="unitSize" label="Rates are">
            {UNIT_SIZES.map(size => (
              <SelectItem key={size.value} value={size.value}>
                {size.label} units
              </SelectItem>
            ))}
          </SelectFormField>
          <SelectFormField control={control} name="onUnmatched" label="Usage no row matches">
            <SelectItem value="hold">Holds the invoice</SelectItem>
            <SelectItem value="ignore">Is not billed</SelectItem>
          </SelectFormField>
        </div>
      )}
    </div>
  )
}

// --- Credit ---

/** "≈ $0.009 per credit (10% off $0.01)": what a sold pack charges per credit. */
const PackCreditPrice = ({
  unit,
  currency,
  packPrice,
  creditsPerPack,
}: {
  unit: CreditUnit
  currency: string
  packPrice: string
  creditsPerPack: string
}) => {
  const value = creditValue(unit, currency)
  const price = packCreditPrice(packPrice, creditsPerPack, value)
  if (!price) return null
  const money = (amount: string) => formatCurrencyNoRounding(amount, currency)
  const versus =
    value && price.discountPct !== undefined && price.discountPct !== 0
      ? price.discountPct > 0
        ? ` (${price.discountPct}% off ${money(value)})`
        : ` (${-price.discountPct}% above ${money(value)})`
      : ''
  return (
    <p className="text-xs text-muted-foreground">
      ≈ {money(price.perCredit)} per {unit.name}
      {versus}
    </p>
  )
}

const CREDIT_GRANT_OPTIONS = [
  { value: 'period', label: 'Every period', hint: 'On each billing period.' },
  { value: 'once', label: 'Once at start', hint: 'On the first invoice only.' },
] as const

function CreditPricingFields({
  control,
  currency,
}: {
  control: Control<FieldValues>
  currency: string
}) {
  const expiryUnit = useWatch({ control, name: 'expiryUnit' }) ?? 'NONE'
  const unitPrice = useWatch({ control, name: 'unitPrice' })
  const creditAmount = useWatch({ control, name: 'creditAmount' })
  const included = isZeroAmount(unitPrice)
  const rolloverEnabled = useWatch({ control, name: 'rolloverEnabled' }) ?? false
  const rolloverExpiryUnit = useWatch({ control, name: 'rolloverExpiryUnit' }) ?? 'MONTH'
  const once = useWatch({ control, name: 'once' }) ?? false
  const canRollOver = included && expiryUnit !== 'NONE' && !once
  const rolloverUnavailableReason = !included
    ? 'Rollover is available for included (free) packs only.'
    : once
      ? 'Credit granted once at start cannot roll over.'
      : expiryUnit === 'NONE'
        ? 'Credit that never expires has nothing to roll over.'
        : undefined
  const { setValue, getValues, getFieldState } = useFormContext()
  useEffect(() => {
    if (!canRollOver && getValues('rolloverEnabled')) setValue('rolloverEnabled', false)
  }, [canRollOver, getValues, setValue])
  const grantedUnit = useGrantedCreditUnit(control)
  const creditCurrency = grantedUnit?.code ?? currency
  const creditPrecision = grantedUnit ? grantedUnit.precision : currencyPrecision(currency)
  return (
    <div className="space-y-4">
      <InputFormField
        name="quantity"
        label="Packs included"
        description="How many packs each subscriber gets."
        type="number"
        step={1}
        className="max-w-xs"
        control={control}
      />
      <GenericFormField
        control={control}
        name="unitPrice"
        label="Price per pack"
        render={({ field }) => (
          <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
        )}
      />
      <CreditPackUnitField control={control} currency={currency} />
      <GenericFormField
        control={control}
        name="creditAmount"
        label={grantedUnit ? `${grantedUnit.pluralName} granted per pack` : 'Credit granted per pack'}
        render={({ field }) => (
          <UncontrolledPriceInput
            {...field}
            currency={creditCurrency}
            unitLabel={grantedUnit ? 'credits' : undefined}
            precision={creditPrecision}
            placeholder={creditPrecision === 0 ? '0' : '0.00'}
            className="max-w-xs"
          />
        )}
      />
      {grantedUnit && !included && (
        <PackCreditPrice
          unit={grantedUnit}
          currency={currency}
          packPrice={String(unitPrice ?? '')}
          creditsPerPack={String(creditAmount ?? '')}
        />
      )}
      <FormField
        control={control}
        name="once"
        render={({ field }) => (
          <FormItem className="space-y-2">
            <FormLabel>Grant</FormLabel>
            <FormControl>
              <RadioGroup
                value={field.value ? 'once' : 'period'}
                onValueChange={value => {
                  const toOnce = value === 'once'
                  field.onChange(toOnce)
                  if (toOnce) setValue('rolloverEnabled', false)
                  // Moves the expiry along with the grant mode only while it is the other mode's
                  // default: period credit resets each period, credit granted once never expires.
                  if (
                    !getFieldState('expiryUnit').isDirty &&
                    getValues('expiryUnit') === (toOnce ? 'BILLING_PERIOD' : 'NONE')
                  ) {
                    setValue('expiryUnit', toOnce ? 'NONE' : 'BILLING_PERIOD')
                  }
                }}
                className="grid gap-2 sm:grid-cols-2 max-w-md"
              >
                {CREDIT_GRANT_OPTIONS.map(option => {
                  const id = `credit-grant-${option.value}`
                  return (
                    <Label
                      key={option.value}
                      htmlFor={id}
                      className="flex cursor-pointer items-start gap-2 rounded-md border border-border p-3 font-normal has-[[data-state=checked]]:border-primary"
                    >
                      <RadioGroupItem value={option.value} id={id} className="mt-0.5" />
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
      <SelectFormField control={control} name="expiryUnit" label="Unused credit expires">
        <SelectItem value="NONE">Never</SelectItem>
        <SelectItem value="BILLING_PERIOD">At the end of the billing period</SelectItem>
        <SelectItem value="DAY">Days after {included ? 'the period starts' : 'payment'}</SelectItem>
        <SelectItem value="MONTH">Months after {included ? 'the period starts' : 'payment'}</SelectItem>
      </SelectFormField>
      {(expiryUnit === 'DAY' || expiryUnit === 'MONTH') && (
        <InputFormField
          control={control}
          name="expiryValue"
          label={expiryUnit === 'DAY' ? 'Number of days' : 'Number of months'}
          type="number"
          min={1}
          className="max-w-xs"
        />
      )}
      <SwitchFormField
        control={control}
        name="rolloverEnabled"
        label="Roll over unused credit"
        disabled={!canRollOver}
        description={
          rolloverUnavailableReason ?? 'Leftover credit carries over once and is spent first.'
        }
      />
      {canRollOver && rolloverEnabled && (
        <div className="space-y-4 pl-4 border-l border-border">
          <GenericFormField
            control={control}
            name="rolloverMaxAmount"
            label="Most credit carried per pack"
            description="Leave empty to carry everything left."
            render={({ field }) => (
              <UncontrolledPriceInput
                {...field}
                currency={creditCurrency}
                unitLabel={grantedUnit ? 'credits' : undefined}
                precision={creditPrecision}
                placeholder={creditPrecision === 0 ? '0' : '0.00'}
                className="max-w-xs"
              />
            )}
          />
          <SelectFormField control={control} name="rolloverExpiryUnit" label="Rolled credit expires">
            <SelectItem value="DAY">Days after the original expiry</SelectItem>
            <SelectItem value="MONTH">Months after the original expiry</SelectItem>
          </SelectFormField>
          <InputFormField
            control={control}
            name="rolloverExpiryValue"
            label={rolloverExpiryUnit === 'DAY' ? 'Number of days' : 'Number of months'}
            type="number"
            min={1}
            className="max-w-xs"
          />
        </div>
      )}
      <p className="text-xs text-muted-foreground max-w-md">
        {included
          ? once
            ? 'Credit arrives when the first period is invoiced'
            : 'Credit arrives when each period is invoiced'
          : "Credit arrives when the pack's invoice is paid"}
        {expiryUnit === 'BILLING_PERIOD' ? ' and expires at period end.' : '.'}
        {!included && ' Credit never pays for a pack.'}
      </p>
    </div>
  )
}
