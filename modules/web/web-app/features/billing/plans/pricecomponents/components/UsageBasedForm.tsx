import {
  SelectFormField,
  InputFormField,
  GenericFormField,
  Button,
  Input,
  SelectItem,
  Form,
  ComboboxFormField,
} from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { useAtom } from 'jotai'
import { PlusIcon, XIcon } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { useFieldArray, useWatch } from 'react-hook-form'
import { useNavigate } from 'react-router-dom'
import { match } from 'ts-pattern'

import { AccordionPanel } from '@/components/AccordionPanel'
import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { SimpleTable } from '@/components/table/SimpleTable'
import {
  componentFeeAtom,
  FeeFormProps,
  EditPriceComponentCard,
} from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { useCurrency } from '@/features/billing/plans/pricecomponents/utils'
import { useZodForm, Methods } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  UsageBasedSchema,
  UsageBased,
  UsagePricingModelType,
  TieredAndVolumeRow,
} from '@/lib/schemas/plans'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { useTypedParams } from '@/utils/params'

// type UsagePricingModelType = "per_unit" | "tiered" | "volume" | "package"

const models: [UsagePricingModelType, string][] = [
  ['per_unit', 'Per unit'],
  ['tiered', 'Tiered'],
  ['volume', 'Volume'],
  ['package', 'Package'],
]

export const UsageBasedForm = (props: FeeFormProps) => {
  const [component] = useAtom(componentFeeAtom)
  const navigate = useNavigate()

  const methods = useZodForm({
    schema: UsageBasedSchema,
    defaultValues: component?.data as UsageBased,
  })

  const { familyExternalId } = useTypedParams<{ familyExternalId: string }>()

  const metrics = useQuery(
    listBillableMetrics,
    {
      familyExternalId: familyExternalId!,
    },
    {
      enabled: !!familyExternalId,
    }
  )

  const metricsOptions =
    metrics.data?.billableMetrics?.map(metric => ({ label: metric.name, value: metric.id })) ?? []

  console.log('errors', methods.formState.errors)
  console.log('values', methods.getValues())

  return (
    <>
      <Form {...methods}>
        <EditPriceComponentCard submit={methods.handleSubmit(props.onSubmit)} cancel={props.cancel}>
          <div className="grid grid-cols-3 gap-2">
            <div className="col-span-1 pr-5 border-r border-border space-y-4">
              <ComboboxFormField
                name="metric.id"
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
  methods: Methods<typeof UsageBasedSchema> // TODO
}) => {
  const model = useWatch({
    control: methods.control,
    name: 'model.model',
  })

  return match(model)
    .with('per_unit', () => <PerUnitForm methods={methods} />)
    .with('tiered', () => <TieredForm methods={methods} />)
    .with('volume', () => <VolumeForm methods={methods} />)
    .with('package', () => <PackageForm methods={methods} />)
    .exhaustive()
}

const PerUnitForm = ({
  methods,
}: {
  methods: Methods<typeof UsageBasedSchema> // TODO
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
  methods: Methods<typeof UsageBasedSchema> // TODO
}) => {
  const currency = useCurrency()
  return <TierTable methods={methods} currency={currency} />
}

const VolumeForm = ({
  methods,
}: {
  methods: Methods<typeof UsageBasedSchema> // TODO
}) => {
  const currency = useCurrency()
  return <TierTable methods={methods} currency={currency} />
}

const PackageForm = ({
  methods,
}: {
  methods: Methods<typeof UsageBasedSchema> // TODO
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
        name="model.data.blockPrice"
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
  methods: Methods<typeof UsageBasedSchema> // TODO
  currency: string
}) => {
  const [shouldInitTiers, setShouldInitTiers] = useState(false)

  const { fields, append, remove } = useFieldArray({
    control: methods.control,
    name: 'model.data.rows',
  })

  // if no tiers, add 2
  useEffect(() => {
    if (fields.length === 0) {
      setShouldInitTiers(true)
    }
  }, [setShouldInitTiers, fields.length])

  useEffect(() => {
    if (shouldInitTiers) {
      append({
        firstUnit: 0,
        unitPrice: '',
      })
      append({
        firstUnit: 100,
        unitPrice: '',
      })
    }
  }, [append, shouldInitTiers])

  const addTier = () => {
    const tiers = [...fields]

    const firstUnit = tiers.length === 0 ? 0 : (tiers[tiers.length - 1]?.lastUnit ?? 0) + 1

    append({
      firstUnit,
      unitPrice: '',
    })
  }

  const removeTier = (idx: number) => {
    remove(idx)
  }

  const columns = useMemo<ColumnDef<TieredAndVolumeRow>[]>(() => {
    return [
      {
        header: 'First unit ',
        cell: ({ row }) => <FirstUnitField methods={methods} rowIndex={row.index} />,
      },
      {
        header: 'Last unit ',
        cell: ({ row }) => <LastUnitInput methods={methods} rowIndex={row.index} />,
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
        cell: ({ row }) => (
          <Button variant="link" size="icon" onClick={() => removeTier(row.index)}>
            <XIcon size={12} />
          </Button>
        ),
      },
    ]
  }, [methods])

  return (
    <>
      <SimpleTable columns={columns} data={fields} />
      <Button variant="link" onClick={addTier}>
        + Add tier
      </Button>
    </>
  )
}

const FirstUnitField = ({
  methods,
  rowIndex,
}: {
  methods: Methods<typeof UsageBasedSchema>
  rowIndex: number
}) => {
  const { setValue, control } = methods
  const prevRowValue = useWatch({
    control,
    name: `model.data.rows.${rowIndex - 1}`,
  })
  const thisValue = useWatch({
    control,
    name: `model.data.rows.${rowIndex}.firstUnit`,
  })

  useEffect(() => {
    const updatedValue = prevRowValue
      ? Math.max(prevRowValue.firstUnit, prevRowValue.lastUnit ?? 0)
      : 0
    setValue(`model.data.rows.${rowIndex}.firstUnit`, updatedValue)
  }, [prevRowValue, rowIndex, setValue])

  return thisValue
}

const LastUnitInput = ({
  methods,
  rowIndex,
}: {
  methods: Methods<typeof UsageBasedSchema>
  rowIndex: number
}) => {
  const { setValue, control } = methods
  const nextRow = useWatch({
    control,
    name: `model.data.rows.${rowIndex + 1}`,
  })
  const thisRow = useWatch({
    control,
    name: `model.data.rows.${rowIndex}`,
  })

  useEffect(() => {
    if (nextRow && !thisRow.lastUnit) {
      // and is not focused todo
      const updatedValue = thisRow.firstUnit + 1
      setValue(`model.data.rows.${rowIndex}.lastUnit`, updatedValue)
    } else if (!nextRow) {
      setValue(`model.data.rows.${rowIndex}.lastUnit`, undefined)
    }
  }, [nextRow, setValue])

  const isLast = !nextRow

  return isLast ? (
    '∞'
  ) : (
    <Input
      type="number"
      {...methods.register(`model.data.rows.${rowIndex}.lastUnit`, {
        setValueAs: (value: string) => {
          const parsed = value === '' ? undefined : parseInt(value)
          if (!parsed || isNaN(parsed)) {
            return undefined
          } else {
            return parsed
          }
        },
      })}
      {...methods.withError(`model.data.rows.${rowIndex}.lastUnit`)}
      placeholder="∞"
    />
  )
}
