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
} from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { PlusIcon, XIcon } from 'lucide-react'
import { memo, useCallback, useEffect, useMemo, useState } from 'react'
import { Control, FieldValues, useFieldArray, useFormContext, useWatch } from 'react-hook-form'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { SimpleTable } from '@/components/table/SimpleTable'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatCurrencyNoRounding } from '@/lib/utils/numbers'

import { CreditPackUnitField, useGrantedCreditUnit } from './CreditPackUnitField'
import { creditValue, packCreditPrice } from './creditValue'
import { PricingType, isZeroAmount, type TierRowSchema } from './schemas'

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
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
  /** For matrix pricing: dimension headers to display */
  dimensionHeaders?: string[]
  /** For matrix pricing: valid dimension combinations from the product's metric */
  validCombinations?: DimensionCombination[]
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
  dimensionHeaders,
  validCombinations,
}: PricingFieldsProps) {
  switch (pricingType) {
    case 'rate':
      return <RatePricingFields control={control} currency={currency} />
    case 'slot':
      return <SlotPricingFields control={control} currency={currency} />
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
    case 'matrix':
      return (
        <MatrixPricingFields
          control={control}
          currency={currency}
          dimensionHeaders={dimensionHeaders}
          validCombinations={validCombinations}
        />
      )
    case 'extraRecurring':
      return <ExtraRecurringPricingFields control={control} currency={currency} />
    case 'oneTime':
      return <OneTimePricingFields control={control} currency={currency} />
    case 'credit':
      return <CreditPricingFields control={control} currency={currency} />
  }
}

// --- Rate ---

function RatePricingFields({
  control,
  currency,
}: {
  control: Control<FieldValues>
  currency: string
}) {
  return (
    <GenericFormField
      control={control}
      name="rate"
      label="Rate"
      render={({ field }) => (
        <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
      )}
    />
  )
}

// --- Slot ---

function SlotPricingFields({
  control,
  currency,
}: {
  control: Control<FieldValues>
  currency: string
}) {
  return (
    <div className="space-y-4">
      <GenericFormField
        control={control}
        name="unitRate"
        label="Price per unit"
        render={({ field }) => (
          <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
        )}
      />
      <div className="grid grid-cols-2 gap-4">
        <InputFormField
          name="minSlots"
          label="Min slots"
          type="number"
          placeholder="Optional"
          control={control}
        />
        <InputFormField
          name="maxSlots"
          label="Max slots"
          type="number"
          placeholder="Optional"
          control={control}
        />
      </div>
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

// --- Matrix ---

const areDimensionsEqual = (d1: DimensionCombination, d2: DimensionCombination): boolean => {
  if (d1.dimension1.key !== d2.dimension1.key || d1.dimension1.value !== d2.dimension1.value) {
    return false
  }
  if (!d1.dimension2 && !d2.dimension2) return true
  if (!d1.dimension2 || !d2.dimension2) return false
  return d1.dimension2.key === d2.dimension2.key && d1.dimension2.value === d2.dimension2.value
}

interface MatrixRow {
  perUnitPrice: string
  dimension1: MatrixDimension
  dimension2?: MatrixDimension
}

function MatrixPricingFields({
  control,
  currency,
  dimensionHeaders,
  validCombinations,
}: {
  control: Control<FieldValues>
  currency: string
  dimensionHeaders?: string[]
  validCombinations?: DimensionCombination[]
}) {
  const { fields, replace } = useFieldArray({
    control,
    name: 'rows',
  })
  const { getValues } = useFormContext()

  // Sync rows with validCombinations: keep prices for matching combos, drop orphans, add new.
  useEffect(() => {
    if (!validCombinations) return

    const currentRows = (getValues('rows') ?? []) as MatrixRow[]
    const synced = validCombinations.map(combo => {
      const existing = currentRows.find(row =>
        areDimensionsEqual(row as unknown as DimensionCombination, combo)
      )
      return { ...combo, perUnitPrice: existing?.perUnitPrice ?? '0' }
    })

    const isSame =
      synced.length === currentRows.length &&
      synced.every((row, i) =>
        areDimensionsEqual(
          row as DimensionCombination,
          currentRows[i] as unknown as DimensionCombination
        ) && row.perUnitPrice === currentRows[i].perUnitPrice
      )
    if (!isSame) replace(synced)
  }, [validCombinations, replace, getValues])

  const isOrphaned = useCallback(
    (row: DimensionCombination) => {
      if (!validCombinations?.length) return false
      return !validCombinations.some(combo => areDimensionsEqual(combo, row))
    },
    [validCombinations]
  )

  const headers = useMemo(() => dimensionHeaders ?? ['Dimension 1'], [dimensionHeaders])

  const columns = useMemo<ColumnDef<MatrixRow>[]>(
    () => [
      {
        header: headers[0] || 'Dimension 1',
        cell: ({ row }) => {
          const orphaned = isOrphaned(row.original as DimensionCombination)
          return (
            <span
              className={orphaned ? 'text-muted-foreground line-through' : ''}
              title={
                orphaned ? 'This dimension value no longer exists in the metric definition' : ''
              }
            >
              {row.original.dimension1?.value}
            </span>
          )
        },
      },
      ...((headers[1]
        ? [
            {
              header: headers[1],
              cell: ({ row }: { row: { original: MatrixRow } }) => {
                const orphaned = isOrphaned(row.original as DimensionCombination)
                return (
                  <span
                    className={orphaned ? 'text-muted-foreground line-through' : ''}
                    title={
                      orphaned
                        ? 'This dimension value no longer exists in the metric definition'
                        : ''
                    }
                  >
                    {row.original.dimension2?.value}
                  </span>
                )
              },
            },
          ]
        : []) as ColumnDef<MatrixRow>[]),
      {
        header: 'Unit price',
        cell: ({ row }) => {
          const orphaned = isOrphaned(row.original as DimensionCombination)
          return (
            <div className={orphaned ? 'opacity-50' : ''}>
              <GenericFormField
                control={control}
                name={`rows.${row.index}.perUnitPrice`}
                render={({ field }) => (
                  <UncontrolledPriceInput
                    {...field}
                    currency={currency}
                    showCurrency={true}
                    precision={8}
                  />
                )}
              />
            </div>
          )
        },
      },
    ],
    [headers, control, currency, isOrphaned]
  )

  if (fields.length === 0) {
    return (
      <p className="text-sm text-muted-foreground py-4">
        No matrix dimensions configured. Add dimension values in the product configuration.
      </p>
    )
  }

  return <SimpleTable columns={columns} data={fields as unknown as MatrixRow[]} />
}

// --- Extra Recurring ---

function ExtraRecurringPricingFields({
  control,
  currency,
}: {
  control: Control<FieldValues>
  currency: string
}) {
  return (
    <div className="space-y-4">
      <InputFormField
        name="quantity"
        label="Quantity"
        type="number"
        step={1}
        className="max-w-xs"
        control={control}
      />
      <GenericFormField
        control={control}
        name="unitPrice"
        label="Unit price"
        render={({ field }) => (
          <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
        )}
      />
    </div>
  )
}

// --- One-Time ---

function OneTimePricingFields({
  control,
  currency,
}: {
  control: Control<FieldValues>
  currency: string
}) {
  return (
    <div className="space-y-4">
      <InputFormField
        name="quantity"
        label="Quantity"
        type="number"
        step={1}
        className="max-w-xs"
        control={control}
      />
      <GenericFormField
        control={control}
        name="unitPrice"
        label="Unit price"
        render={({ field }) => (
          <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
        )}
      />
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
