import {
  Button,
  ComboboxFormField,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormMessage,
  GenericFormField,
  Input,
  InputFormField,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { useAtom, useSetAtom } from 'jotai'
import { PlusIcon, XIcon } from 'lucide-react'
import { memo, useCallback, useEffect, useMemo, useState } from 'react'
import { useFieldArray, useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { match } from 'ts-pattern'

import { AccordionPanel } from '@/components/AccordionPanel'
import PriceInput, { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { SimpleTable } from '@/components/table/SimpleTable'
import { EditPriceComponentCard } from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { useCurrency } from '@/features/billing/plans/pricecomponents/utils'
import { Methods, useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  Dimension,
  Matrix,
  TieredAndVolumeRow,
  UsageFee,
  UsageFeeSchema,
  UsagePricingModelType,
} from '@/lib/schemas/plans'
import {
  getBillableMetric,
  listBillableMetrics,
} from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { useTypedParams } from '@/utils/params'

import { componentFeeAtom, componentNameAtom } from '../atoms'

import { FeeFormProps } from './shared'

// type UsagePricingModelType = "per_unit" | "tiered" | "volume" | "package"

const models: [UsagePricingModelType, string][] = [
  ['per_unit', 'Per unit'],
  ['tiered', 'Tiered'],
  ['volume', 'Volume'],
  ['package', 'Package'],
  ['matrix', 'Matrix'],
]

const MetricSetter = ({
  methods,
  metricsOptions,
}: {
  methods: Methods<typeof UsageFeeSchema>
  metricsOptions: {
    label: string
    value: string
  }[]
}) => {
  const metricId = useWatch({
    control: methods.control,
    name: 'metricId',
  })

  const setName = useSetAtom(componentNameAtom)

  useEffect(() => {
    const metric = metricsOptions.find(m => m.value === metricId)
    metric?.label && setName(metric.label)
  }, [setName, metricId, metricsOptions])

  return null
}

export const UsageBasedForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const navigate = useNavigate()

  const methods = useZodForm({
    schema: UsageFeeSchema,
    defaultValues: component?.data as UsageFee,
  })

  const { familyLocalId } = useTypedParams<{ familyLocalId: string }>()

  const metrics = useQuery(
    listBillableMetrics,
    {
      familyLocalId: familyLocalId!,
    },
    {
      enabled: !!familyLocalId,
    }
  )

  const metricsOptions =
    metrics.data?.billableMetrics?.map(metric => ({ label: metric.name, value: metric.id })) ?? []

  console.log('errors', methods.formState.errors)
  console.log('values', methods.getValues())

  return (
    <>
      <Form {...methods}>
        <MetricSetter methods={methods} metricsOptions={metricsOptions} />
        <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
          <div className="grid grid-cols-3 gap-2">
            <div className="col-span-1 pr-5 border-r border-border space-y-4">
              <ComboboxFormField
                name="metricId"
                label="Billable metric"
                control={methods.control}
                placeholder="Select a metric"
                options={metricsOptions}
                // empty={!metricsOptions.length}
                action={
                  <Button
                    hasIcon
                    variant="ghost"
                    size="full"
                    onClick={() => navigate('add-metric')}
                  >
                    <PlusIcon size={12} /> New metric
                  </Button>
                }
              />
            </div>
            <div className="ml-4 col-span-2 space-y-4">
              <SelectFormField
                name="model.model"
                label="Pricing model"
                placeholder="Select a model"
                className="max-w-[320px]"
                empty={models.length === 0}
                control={methods.control}
              >
                {models.map(([option, label]) => (
                  <SelectItem value={option} key={option}>
                    {label}
                  </SelectItem>
                ))}
              </SelectFormField>
              <UsageBasedDataForm methods={methods} />
            </div>
          </div>
        </EditPriceComponentCard>
      </Form>
    </>
  )
}

const UsageBasedDataForm = ({
  methods,
}: {
  methods: Methods<typeof UsageFeeSchema> // TODO
}) => {
  const model = useWatch({
    control: methods.control,
    name: 'model.model',
  })

  return match(model)
    .with('matrix', () => <MatrixForm methods={methods} />)
    .with('per_unit', () => <PerUnitForm methods={methods} />)
    .with('tiered', () => <TieredForm methods={methods} />)
    .with('volume', () => <VolumeForm methods={methods} />)
    .with('package', () => <PackageForm methods={methods} />)
    .exhaustive()
}

type DimensionCombination = { dimension1: Dimension; dimension2?: Dimension }

// Helper function to compare dimensions
const areDimensionsEqual = (dim1: DimensionCombination, dim2: DimensionCombination): boolean => {
  return (
    dim1.dimension1.key === dim2.dimension1.key &&
    dim1.dimension1.value === dim2.dimension1.value &&
    (!dim1.dimension2 ||
      !dim2.dimension2 ||
      (dim1.dimension2.key === dim2.dimension2.key &&
        dim1.dimension2.value === dim2.dimension2.value))
  )
}

const MatrixForm = ({ methods }: { methods: Methods<typeof UsageFeeSchema> }) => {
  const currency = useCurrency()

  const { fields, append, remove } = useFieldArray({
    control: methods.control,
    name: 'model.data.dimensionRates',
  })

  const [dimensionHeaders, setDimensionHeaders] = useState<string[]>([])

  const metricId = useWatch({
    control: methods.control,
    name: 'metricId',
  })

  const metric = useQuery(getBillableMetric, { id: metricId }, { enabled: !!metricId })?.data

  useEffect(() => {
    if (!metric?.billableMetric?.segmentationMatrix) return

    const segmentationMatrix = metric.billableMetric.segmentationMatrix
    let headers: string[] = []
    let dimensionCombinations: {
      dimension1: { key: string; value: string }
      dimension2?: { key: string; value: string }
    }[] = []

    match(segmentationMatrix.matrix)
      .with({ case: 'single' }, ({ value }) => {
        headers = [value.dimension?.key ?? '']
        dimensionCombinations = (value.dimension?.values ?? []).map(v => ({
          dimension1: { key: headers[0], value: v },
        }))
      })
      .with({ case: 'double' }, ({ value }) => {
        headers = [value.dimension1?.key ?? '', value.dimension2?.key ?? '']
        dimensionCombinations = (value.dimension1?.values ?? []).flatMap(v1 =>
          (value.dimension2?.values ?? []).map(v2 => ({
            dimension1: { key: headers[0], value: v1 },
            dimension2: { key: headers[1], value: v2 },
          }))
        )
      })
      .with({ case: 'linked' }, ({ value }) => {
        headers = [value.dimensionKey, value.linkedDimensionKey]
        dimensionCombinations = Object.entries(value.values).flatMap(([k, v]) =>
          v.values.map(linkedV => ({
            dimension1: { key: headers[0], value: k },
            dimension2: { key: headers[1], value: linkedV },
          }))
        )
      })
      .otherwise(() => {})

    setDimensionHeaders(headers)

    // Update or create rows based on the current state
    const currentDimensions = fields.map(field => ({
      dimension1: field.dimension1,
      dimension2: field.dimension2,
    }))
    const newRows = dimensionCombinations.filter(
      combo => !currentDimensions.some(dim => areDimensionsEqual(dim, combo))
    )
    const removedRows = currentDimensions.filter(
      dim => !dimensionCombinations.some(combo => areDimensionsEqual(dim, combo))
    )
    newRows.forEach(dimensions => {
      append({ ...dimensions, price: '0' })
    })

    removedRows.forEach(dimensions => {
      const index = fields.findIndex(field => areDimensionsEqual(field, dimensions))
      if (index !== -1) remove(index)
    })
  }, [metric, fields, append, remove])

  const columns = useMemo<ColumnDef<Matrix['dimensionRates'][number]>[]>(
    () => [
      {
        header: dimensionHeaders[0] || 'Dimension 1',
        accessorFn: row => row.dimension1.value,
      },
      ...((dimensionHeaders[1]
        ? [
            {
              header: dimensionHeaders[1],
              accessorFn: row => row.dimension2?.value,
            },
          ]
        : []) as ColumnDef<Matrix['dimensionRates'][number]>[]),
      {
        header: 'Unit price',
        accessor: 'price',
        cell: ({ row }) => (
          <PriceInput
            {...methods.withControl(`model.data.dimensionRates.${row.index}.price`)}
            {...methods.withError(`model.data.dimensionRates.${row.index}.price`)}
            currency={currency}
            showCurrency={true}
            precision={8}
          />
        ),
      },
    ],
    [dimensionHeaders, methods, currency]
  )

  if (!metric?.billableMetric) return null

  const segmentationMatrix = metric.billableMetric.segmentationMatrix

  if (!segmentationMatrix) {
    return (
      <div className="py-4 text-sm text-muted-foreground">
        This metric does not have a segmentation matrix
      </div>
    )
  }

  return (
    <>
      <SimpleTable columns={columns} data={fields} />
    </>
  )
}

const PerUnitForm = ({
  methods,
}: {
  methods: Methods<typeof UsageFeeSchema> // TODO
}) => {
  const currency = useCurrency()

  return (
    <>
      <GenericFormField
        control={methods.control}
        name="model.data.unitPrice"
        label="Price per unit"
        render={({ field }) => (
          <UncontrolledPriceInput
            {...field}
            currency={currency}
            className="max-w-xs"
            precision={8}
          />
        )}
      />

      <div className="w-full border-b border-border pt-4"></div>

      <AccordionPanel
        title={
          <div className="space-x-4 items-center flex pr-4 text-muted-foreground">Adjustments</div>
        }
        defaultOpen={false}
        triggerClassName="justify-normal"
      >
        <div className="space-y-6">Included</div>
      </AccordionPanel>
    </>
  )
}

const TieredForm = ({
  methods,
}: {
  methods: Methods<typeof UsageFeeSchema> // TODO
}) => {
  const currency = useCurrency()
  return <TierTable methods={methods} currency={currency} />
}

const VolumeForm = ({
  methods,
}: {
  methods: Methods<typeof UsageFeeSchema> // TODO
}) => {
  const currency = useCurrency()
  return <TierTable methods={methods} currency={currency} />
}

const PackageForm = ({
  methods,
}: {
  methods: Methods<typeof UsageFeeSchema> // TODO
}) => {
  const currency = useCurrency()
  return (
    <>
      <InputFormField
        name="model.data.blockSize"
        label="Block size"
        type="number"
        step={1}
        className="max-w-xs"
        control={methods.control}
      />

      <GenericFormField
        control={methods.control}
        name="model.data.packagePrice"
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
    </>
  )
}

const TierTable = ({
  methods,
  currency,
}: {
  methods: Methods<typeof UsageFeeSchema>
  currency: string
}) => {
  const { fields, append, remove } = useFieldArray({
    control: methods.control,
    name: 'model.data.rows',
  })

  const [shouldInitTiers, setShouldInitTiers] = useState(false)

  useEffect(() => {
    if (fields.length === 0) {
      setShouldInitTiers(true)
    }
  }, [fields.length])

  useEffect(() => {
    if (shouldInitTiers) {
      append({ firstUnit: BigInt(0), unitPrice: '' })
      append({ firstUnit: BigInt(100), unitPrice: '' })
      setShouldInitTiers(false)
    }
  }, [shouldInitTiers, append])

  const addTier = useCallback(() => {
    const lastTier = fields[fields.length - 1]
    const firstUnit = lastTier ? BigInt(lastTier.firstUnit) + BigInt(1) : BigInt(0)
    append({ firstUnit, unitPrice: '' })
  }, [fields, append])

  const columns = useMemo<ColumnDef<TieredAndVolumeRow>[]>(
    () => [
      {
        header: 'First unit',
        cell: ({ row }) => <FirstUnitField methods={methods} rowIndex={row.index} />,
      },
      {
        header: 'Last unit',
        cell: ({ row }) => <LastUnitCell methods={methods} rowIndex={row.index} />,
      },
      {
        header: 'Per unit',
        cell: ({ row }) => (
          <GenericFormField
            control={methods.control}
            name={`model.data.rows.${row.index}.unitPrice`}
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
      {
        header: '',
        id: 'remove',
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
    [methods, currency, fields.length, remove]
  )

  return (
    <>
      <SimpleTable columns={columns} data={fields} />
      <Button variant="link" onClick={addTier}>
        + Add tier
      </Button>
    </>
  )
}

const FirstUnitField = memo(
  ({ methods, rowIndex }: { methods: Methods<typeof UsageFeeSchema>; rowIndex: number }) => {
    const { control } = methods
    const prevRowValue = useWatch({
      control,
      name: `model.data.rows.${rowIndex - 1}`,
    })

    const isFirst = rowIndex === 0

    return (
      <>
        <FormField
          control={control}
          name={`model.data.rows.${rowIndex}.firstUnit`}
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
                  onChange={e => {
                    const value = e.target.value
                    field.onChange(BigInt(value))
                  }}
                  value={Number(field.value)}
                  disabled={isFirst}
                  placeholder={isFirst ? '0' : ''}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
      </>
    )
  }
)

const LastUnitCell = ({
  methods,
  rowIndex,
}: {
  methods: Methods<typeof UsageFeeSchema>
  rowIndex: number
}) => {
  const nextRow = useWatch({
    control: methods.control,
    name: `model.data.rows.${rowIndex + 1}`,
  })

  const isLast = !nextRow

  return isLast ? '∞' : `${BigInt(nextRow.firstUnit)}`
}
