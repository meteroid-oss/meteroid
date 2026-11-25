import { Button, ComboboxFormField, InputFormField, SelectFormField, SelectItem } from '@md/ui'
import { ExternalLinkIcon, PlusIcon } from 'lucide-react'
import { useMemo } from 'react'

import { usePlanOverview } from '@/features/plans/hooks/usePlan'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'

import type { ComponentFeeType } from '@/features/pricing/conversions'
import type { Methods } from '@/hooks/useZodForm'
import type { z } from 'zod'

// What a new product is, beside its prices: the metric it counts, its seat unit, its billing.

export const EditableStructuralFields = ({
  feeType,
  methods,
  disabled,
  familyLocalId,
}: {
  feeType: ComponentFeeType
  methods: Methods<z.ZodType>
  disabled?: boolean
  familyLocalId?: string
}) => {
  switch (feeType) {
    case 'slot':
      return (
        <InputFormField
          name="slotUnitName"
          label="Slot unit name"
          control={methods.control}
          className="max-w-xs"
          disabled={disabled}
        />
      )
    case 'capacity':
      return <MetricCombobox methods={methods} disabled={disabled} familyLocalId={familyLocalId} />
    case 'usage':
      return (
        <UsageEditableStructural
          methods={methods}
          disabled={disabled}
          familyLocalId={familyLocalId}
        />
      )
    case 'extraRecurring':
      return (
        <SelectFormField
          name="billingType"
          label="Billing type"
          control={methods.control}
          className="max-w-xs"
          disabled={disabled}
        >
          <SelectItem value="ADVANCE">Paid upfront (advance)</SelectItem>
          <SelectItem value="ARREAR">Postpaid (arrear)</SelectItem>
        </SelectFormField>
      )
    default:
      return null
  }
}

const UsageEditableStructural = ({
  methods,
  disabled,
  familyLocalId,
}: {
  methods: Methods<z.ZodType>
  disabled?: boolean
  familyLocalId?: string
}) => {
  const usageModels: [string, string][] = [
    ['per_unit', 'Per unit'],
    ['tiered', 'Tiered'],
    ['volume', 'Volume'],
    ['package', 'Package'],
    ['dimension_rates', 'By dimension'],
    ['rate_card', 'By dimension, from a rate card'],
  ]

  return (
    <div className="grid grid-cols-2 gap-3 items-end">
      <MetricCombobox methods={methods} disabled={disabled} familyLocalId={familyLocalId} />
      <SelectFormField
        name="usageModel"
        label="Usage model"
        control={methods.control}
        placeholder="Select model"
        disabled={disabled}
      >
        {usageModels.map(([value, label]) => (
          <SelectItem key={value} value={value}>
            {label}
          </SelectItem>
        ))}
      </SelectFormField>
    </div>
  )
}

export const MetricCombobox = ({
  methods,
  disabled,
  familyLocalId: familyLocalIdProp,
}: {
  methods: Methods<z.ZodType>
  disabled?: boolean
  familyLocalId?: string
}) => {
  const basePath = useBasePath()
  const plan = usePlanOverview()
  const familyLocalId = familyLocalIdProp ?? plan?.productFamilyLocalId

  const metricsQuery = useQuery(listBillableMetrics, familyLocalId ? { familyLocalId } : {}, {
    refetchOnWindowFocus: 'always',
  })

  const options = useMemo(
    () => metricsQuery.data?.billableMetrics?.map(m => ({ label: m.name, value: m.id })) ?? [],
    [metricsQuery.data]
  )

  return (
    <ComboboxFormField
      name="metricId"
      label="Billable metric"
      control={methods.control}
      placeholder="Select a metric"
      options={options}
      disabled={disabled}
      action={
        !disabled ? (
          <Button
            hasIcon
            variant="ghost"
            size="full"
            onClick={() => window.open(`${basePath}/metrics/add-metric`, '_blank')}
          >
            <PlusIcon size={12} /> New metric <ExternalLinkIcon size={10} />
          </Button>
        ) : undefined
      }
    />
  )
}

/** An existing product's metric, read-only. */
export const MetricDisplay = ({
  metricId,
  familyLocalId: familyLocalIdProp,
}: {
  metricId?: string
  familyLocalId?: string
}) => {
  const plan = usePlanOverview()
  const familyLocalId = familyLocalIdProp ?? plan?.productFamilyLocalId
  const metricsQuery = useQuery(listBillableMetrics, familyLocalId ? { familyLocalId } : {}, {
    refetchOnWindowFocus: 'always',
  })
  const metric = metricsQuery.data?.billableMetrics?.find(m => m.id === metricId)

  if (!metricId || !metric) return null
  return (
    <div className="space-y-1">
      <span className="peer-disabled:cursor-not-allowed peer-disabled:opacity-70 dark:text-muted-foreground font-normal text-xs">
        Metric
      </span>
      <div className="text-xs rounded-md py-1.5">{metric.name}</div>
    </div>
  )
}
