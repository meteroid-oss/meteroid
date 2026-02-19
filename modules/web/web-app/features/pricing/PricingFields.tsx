import {
  Button,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuTrigger,
  FormControl,
  FormField,
  FormItem,
  FormMessage,
  GenericFormField,
  Input,
  InputFormField,
} from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { PlusIcon, XIcon } from 'lucide-react'
import { memo, useCallback, useEffect, useMemo, useState } from 'react'
import { Control, FieldValues, useFieldArray, useFormContext, useWatch } from 'react-hook-form'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { SimpleTable } from '@/components/table/SimpleTable'

import { PricingType, type TierRowSchema } from './schemas'

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
  dimensionHeaders,
  validCombinations,
}: PricingFieldsProps) {
  switch (pricingType) {
    case 'rate':
      return <RatePricingFields control={control} currency={currency} />
    case 'slot':
      return <SlotPricingFields control={control} currency={currency} />
    case 'capacity':
      return <CapacityPricingFields control={control} currency={currency} />
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
}: {
  control: Control<FieldValues>
  currency: string
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
        header: 'Overage rate',
        cell: ({ row }) => (
          <GenericFormField
            control={control}
            name={`thresholds.${row.index}.overageRate`}
            render={({ field }) => (
              <UncontrolledPriceInput
                {...field}
                currency={currency}
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
    [control, currency, fields.length, remove]
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
  return (
    d1.dimension1.key === d2.dimension1.key &&
    d1.dimension1.value === d2.dimension1.value &&
    (!d1.dimension2 ||
      !d2.dimension2 ||
      (d1.dimension2.key === d2.dimension2.key && d1.dimension2.value === d2.dimension2.value))
  )
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
  const { fields, append } = useFieldArray({
    control,
    name: 'rows',
  })
  const { getValues } = useFormContext()

  // Auto-populate missing combinations when validCombinations changes
  useEffect(() => {
    if (!validCombinations?.length) return

    const currentRows = (getValues('rows') ?? []) as MatrixRow[]

    const newRows = validCombinations.filter(
      combo =>
        !currentRows.some(row => areDimensionsEqual(row as unknown as DimensionCombination, combo))
    )

    if (newRows.length > 0) {
      append(newRows.map(dimensions => ({ ...dimensions, perUnitPrice: '0' })))
    }
  }, [validCombinations, append, getValues])

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
