import { disableQuery } from '@connectrpc/connect-query'
import {
  Button,
  ComboboxFormField,
  Form,
  GenericFormField,
  InputFormField,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { ExternalLinkIcon, PlusIcon } from 'lucide-react'
import { useEffect, useMemo, useRef } from 'react'
import { useWatch } from 'react-hook-form'
import { z } from 'zod'

import { UncontrolledPriceInput } from '@/components/form/PriceInput'
import { usePlanOverview } from '@/features/plans/hooks/usePlan'
import { PricingFields } from '@/features/pricing/PricingFields'
import { useMatrixDimensions } from '@/hooks/useMatrixDimensions'
import {
  CapacityComponentSchema,
  ExtraRecurringComponentSchema,
  OneTimeComponentSchema,
  RateComponentSchema,
  SlotComponentSchema,
  UsageFormSchema,
} from '@/features/pricing/componentSchemas'
import type { ComponentFeeType } from '@/features/pricing/conversions'
import { pricesToFormData, toPricingTypeFromFeeType } from '@/features/pricing/conversions'
import {
  CapacityPricingSchema,
  ExtraRecurringPricingSchema,
  MatrixPricingSchema,
  OneTimePricingSchema,
  PackagePricingSchema,
  PerUnitPricingSchema,
  RatePricingSchema,
  SlotPricingSchema,
  TierRowSchema,
  pricingDefaults,
} from '@/features/pricing/schemas'
import { useBasePath } from '@/hooks/useBasePath'
import { useZodForm, type Methods } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  getBillableMetric,
  listBillableMetrics,
} from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import type { BillableMetric } from '@/rpc/api/billablemetrics/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

// --- Schemas ---

const Cadence = z.enum(['MONTHLY', 'QUARTERLY', 'SEMIANNUAL', 'ANNUAL'])

// Full component schemas — include structural fields (metricId, billingType, slotUnitName, etc.)
const fullSchemas: Record<ComponentFeeType, z.ZodType> = {
  rate: RateComponentSchema,
  slot: SlotComponentSchema,
  capacity: CapacityComponentSchema,
  usage: UsageFormSchema,
  extraRecurring: ExtraRecurringComponentSchema,
  oneTime: OneTimeComponentSchema,
}

// Pricing-only schemas (library products — no structural fields)
const pricingOnlySchemas: Record<string, z.ZodType> = {
  rate: z.object({ term: Cadence, ...RatePricingSchema.shape }),
  slot: z.object({ term: Cadence, ...SlotPricingSchema.shape }),
  capacity: z.object({
    term: Cadence,
    thresholds: z.array(CapacityPricingSchema).min(1),
  }),
  extraRecurring: z.object({ term: Cadence, ...ExtraRecurringPricingSchema.shape }),
  oneTime: z.object({ ...OneTimePricingSchema.shape }),
}

// Usage pricing-only schemas keyed by usage model — no metricId/usageModel (those live on the product)
const usagePricingOnlySchemas: Record<string, z.ZodType> = {
  per_unit: z.object({ term: Cadence, ...PerUnitPricingSchema.shape }),
  tiered: z.object({
    term: Cadence,
    rows: z.array(TierRowSchema).min(2, 'At least 2 tiers required'),
  }),
  volume: z.object({
    term: Cadence,
    rows: z.array(TierRowSchema).min(2, 'At least 2 tiers required'),
  }),
  package: z.object({ term: Cadence, ...PackagePricingSchema.shape }),
  matrix: z.object({ term: Cadence, ...MatrixPricingSchema.shape }),
}

export function getComponentSchema(
  feeType: ComponentFeeType,
  mode: 'full' | 'pricingOnly',
  usageModel?: string
): z.ZodType {
  if (mode === 'full') return fullSchemas[feeType]
  if (feeType === 'usage') return usagePricingOnlySchemas[usageModel ?? 'per_unit']
  return pricingOnlySchemas[feeType]
}

// --- Helpers ---

export interface StructuralInfo {
  metricId?: string
  usageModel?: string
  slotUnitName?: string
  billingType?: string
}

function deriveUsageModelFromPrice(price?: Price): string | undefined {
  if (!price?.pricing || price.pricing.case !== 'usagePricing') return undefined
  return price.pricing.value.model.case
}

const usageModelProtoToForm: Record<string, string> = {
  perUnit: 'per_unit',
  tiered: 'tiered',
  volume: 'volume',
  package: 'package',
  matrix: 'matrix',
}

export function buildDefaults(
  feeType: ComponentFeeType,
  existingPrice?: Price
): Record<string, unknown> {
  if (!existingPrice) {
    switch (feeType) {
      case 'rate':
        return { term: 'MONTHLY', rate: '0.00' }
      case 'slot':
        return {
          slotUnitName: 'Seats',
          upgradePolicy: 'PRORATED',
          downgradePolicy: 'REMOVE_AT_END_OF_PERIOD',
          minimumCount: 1,
          term: 'MONTHLY',
          unitRate: '0.00',
        }
      case 'capacity':
        return {
          metricId: '',
          term: 'MONTHLY',
          thresholds: [{ included: 0, rate: '0.00', overageRate: '0.00000000' }],
        }
      case 'usage':
        return { metricId: '', usageModel: 'per_unit', term: 'MONTHLY', unitPrice: '0.00000000' }
      case 'extraRecurring':
        return { term: 'MONTHLY', billingType: 'ADVANCE', unitPrice: '0.00', quantity: 1 }
      case 'oneTime':
        return { unitPrice: '0.00', quantity: 1 }
    }
  }

  const usageModel = deriveUsageModelFromPrice(existingPrice)
  const pricingType = toPricingTypeFromFeeType(feeType, usageModel)
  const formData = pricesToFormData([existingPrice], pricingType)

  if (feeType === 'usage' && usageModel) {
    return { ...formData, usageModel: usageModelProtoToForm[usageModel] ?? 'per_unit' }
  }

  return formData
}

export function buildDefaultsFromPrices(
  feeType: ComponentFeeType,
  prices: Price[]
): Record<string, unknown> {
  if (prices.length === 0) return buildDefaults(feeType)

  const usageModel = deriveUsageModelFromPrice(prices[0])
  const pricingType = toPricingTypeFromFeeType(feeType, usageModel)
  const formData = pricesToFormData(prices, pricingType)

  if (feeType === 'usage' && usageModel) {
    return { ...formData, usageModel: usageModelProtoToForm[usageModel] ?? 'per_unit' }
  }

  return formData
}

// --- Shared form content (structural + pricing fields) ---

export const PriceComponentFormContent = ({
  feeType,
  currency,
  methods,
  structural,
  editableStructure,
  isEdit,
  familyLocalId,
}: {
  feeType: ComponentFeeType
  currency: string
  methods: Methods<z.ZodType>
  structural?: StructuralInfo
  editableStructure?: boolean
  isEdit?: boolean
  familyLocalId?: string
}) => (
  <div className="space-y-4">
    {editableStructure ? (
      <EditableStructuralFields
        feeType={feeType}
        methods={methods}
        disabled={isEdit}
        familyLocalId={familyLocalId}
      />
    ) : (
      <StructuralInfoDisplay
        feeType={feeType}
        structural={structural ?? {}}
        familyLocalId={familyLocalId}
      />
    )}
    <PricingFormFields
      feeType={feeType}
      currency={currency}
      methods={methods}
      structural={structural ?? {}}
    />
  </div>
)

// --- Main component (for add flows) ---

interface ProductPricingFormProps {
  feeType: ComponentFeeType
  currency: string
  existingPrice?: Price
  structuralInfo?: StructuralInfo
  editableStructure?: boolean
  onSubmit: (formData: Record<string, unknown>) => void
  submitLabel?: string
  familyLocalId?: string
}

export const ProductPricingForm = ({
  feeType,
  currency,
  existingPrice,
  structuralInfo,
  editableStructure,
  onSubmit,
  submitLabel = 'Add to Plan',
  familyLocalId,
}: ProductPricingFormProps) => {
  const structural = structuralInfo ?? {}

  const defaults = useMemo(() => buildDefaults(feeType, existingPrice), [feeType, existingPrice])

  const schema = useMemo(
    () =>
      getComponentSchema(
        feeType,
        editableStructure ? 'full' : 'pricingOnly',
        structural.usageModel
      ),
    [feeType, editableStructure, structural.usageModel]
  )

  const methods = useZodForm({ schema: schema as z.ZodType, defaultValues: defaults })

  // Reset form when existingPrice changes after mount (e.g., currency resolved async)
  const prevPriceId = useRef(existingPrice?.id)
  useEffect(() => {
    const newId = existingPrice?.id
    if (newId !== prevPriceId.current) {
      prevPriceId.current = newId
      methods.reset(defaults)
    }
  }, [existingPrice?.id, defaults, methods])

  return (
    <Form {...methods}>
      <PriceComponentFormContent
        feeType={feeType}
        currency={currency}
        methods={methods}
        structural={structural}
        editableStructure={editableStructure}
        familyLocalId={familyLocalId}
      />
      <div className="flex justify-end pt-2">
        <Button
          type="button"
          variant="brand"
          onClick={methods.handleSubmit(
            data => onSubmit(data as Record<string, unknown>),
            errors => {
              console.error('Form validation errors:', errors)
              console.error('Current form values:', methods.getValues())
            }
          )}
        >
          {submitLabel}
        </Button>
      </div>
    </Form>
  )
}

// --- Editable structural fields ---

const EditableStructuralFields = ({
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
    ['matrix', 'Matrix'],
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

const MetricCombobox = ({
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

// --- Structural info display (read-only, for library products) ---

function usageModelLabel(model?: string): string {
  switch (model) {
    case 'per_unit':
      return 'Per unit'
    case 'tiered':
      return 'Tiered (Graduated)'
    case 'volume':
      return 'Volume'
    case 'package':
      return 'Package'
    case 'matrix':
      return 'Matrix'
    default:
      return ''
  }
}

const ReadOnlyField = ({ label, value }: { label: string; value: string }) => (
  <div className="space-y-1">
    <span className="peer-disabled:cursor-not-allowed peer-disabled:opacity-70 dark:text-muted-foreground font-normal text-xs">
      {label}
    </span>
    <div className="text-xs rounded-md py-1.5">{value}</div>
  </div>
)

const StructuralInfoDisplay = ({
  feeType,
  structural,
  familyLocalId,
}: {
  feeType: ComponentFeeType
  structural: StructuralInfo
  familyLocalId?: string
}) => {
  switch (feeType) {
    case 'usage':
      return structural.usageModel || structural.metricId ? (
        <div className="grid grid-cols-2 gap-3">
          {structural.usageModel && (
            <ReadOnlyField label="Pricing model" value={usageModelLabel(structural.usageModel)} />
          )}
          {structural.metricId && (
            <MetricDisplay metricId={structural.metricId} familyLocalId={familyLocalId} />
          )}
        </div>
      ) : null
    case 'capacity':
      return structural.metricId ? (
        <MetricDisplay metricId={structural.metricId} familyLocalId={familyLocalId} />
      ) : null
    case 'slot':
      return structural.slotUnitName ? (
        <ReadOnlyField label="Slot unit" value={structural.slotUnitName} />
      ) : null
    case 'extraRecurring':
      return structural.billingType ? (
        <ReadOnlyField
          label="Billing type"
          value={structural.billingType === 'ADVANCE' ? 'Paid upfront' : 'Postpaid'}
        />
      ) : null
    default:
      return null
  }
}

const MetricDisplay = ({
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
  return <ReadOnlyField label="Metric" value={metric.name} />
}

// --- Pricing form fields router ---

const PricingFormFields = ({
  feeType,
  currency,
  methods,
  structural,
}: {
  feeType: ComponentFeeType
  currency: string
  methods: Methods<z.ZodType>
  structural: StructuralInfo
}) => {
  switch (feeType) {
    case 'rate':
      return <RatePricingFields methods={methods} currency={currency} />
    case 'slot':
      return <SlotPricingFields methods={methods} currency={currency} />
    case 'capacity':
      return <CapacityPricingInline methods={methods} currency={currency} />
    case 'usage':
      return <UsagePricingInline methods={methods} currency={currency} structural={structural} />
    case 'extraRecurring':
      return <ExtraRecurringInline methods={methods} currency={currency} />
    case 'oneTime':
      return <PricingFields pricingType="oneTime" control={methods.control} currency={currency} />
  }
}

// --- Cadence select (shared) ---

const CadenceSelect = ({ methods }: { methods: Methods<z.ZodType> }) => (
  <SelectFormField
    name="term"
    label="Cadence"
    control={methods.control}
    placeholder="Select cadence"
  >
    <SelectItem value="MONTHLY">Monthly</SelectItem>
    <SelectItem value="QUARTERLY">Quarterly</SelectItem>
    <SelectItem value="SEMIANNUAL">Semiannual</SelectItem>
    <SelectItem value="ANNUAL">Annual</SelectItem>
  </SelectFormField>
)

// --- Rate pricing (single cadence) ---

const RatePricingFields = ({
  methods,
  currency,
}: {
  methods: Methods<z.ZodType>
  currency: string
}) => (
  <div className="space-y-4">
    <CadenceSelect methods={methods} />
    <GenericFormField
      control={methods.control}
      name="rate"
      label="Rate"
      render={({ field }) => (
        <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
      )}
    />
  </div>
)

// --- Slot pricing (single cadence) ---

const SlotPricingFields = ({
  methods,
  currency,
}: {
  methods: Methods<z.ZodType>
  currency: string
}) => (
  <div className="space-y-4">
    <CadenceSelect methods={methods} />
    <GenericFormField
      control={methods.control}
      name="unitRate"
      label="Price per unit"
      render={({ field }) => (
        <UncontrolledPriceInput {...field} currency={currency} className="max-w-xs" />
      )}
    />
  </div>
)

// --- Capacity inline ---

const CapacityPricingInline = ({
  methods,
  currency,
}: {
  methods: Methods<z.ZodType>
  currency: string
}) => (
  <div className="space-y-4">
    <CadenceSelect methods={methods} />
    <PricingFields pricingType="capacity" control={methods.control} currency={currency} />
  </div>
)

// --- Usage inline ---

const pricingFieldKeys = ['unitPrice', 'rows', 'packagePrice', 'blockSize'] as const

const UsagePricingInline = ({
  methods,
  currency,
  structural,
}: {
  methods: Methods<z.ZodType>
  currency: string
  structural: StructuralInfo
}) => {
  const usageModel =
    useWatch({ control: methods.control, name: 'usageModel' }) ??
    structural.usageModel ??
    'per_unit'
  const metricId = useWatch({ control: methods.control, name: 'metricId' }) ?? structural.metricId
  const pricingType = toPricingTypeFromFeeType('usage', usageModel)

  const prevModel = useRef(usageModel)
  useEffect(() => {
    if (usageModel && usageModel !== prevModel.current) {
      const newType = toPricingTypeFromFeeType('usage', usageModel)
      const defaults = pricingDefaults(newType)
      for (const key of pricingFieldKeys) {
        methods.setValue(key, key === 'rows' ? [] : undefined, { shouldValidate: false })
      }
      for (const [key, val] of Object.entries(defaults)) {
        methods.setValue(key, val, { shouldValidate: false })
      }
      prevModel.current = usageModel
    }
  }, [usageModel, methods])

  const metricQuery = useQuery(getBillableMetric, metricId ? { id: metricId } : disableQuery)
  const { dimensionHeaders, validCombinations } = useMatrixDimensions(
    usageModel === 'matrix' ? metricQuery.data?.billableMetric : undefined
  )

  const showMatrixEmpty =
    usageModel === 'matrix' &&
    (!metricId || (metricQuery.data && !metricQuery.data.billableMetric?.segmentationMatrix))

  return (
    <div className="space-y-4">
      <CadenceSelect methods={methods} />
      {showMatrixEmpty ? (
        <p className="text-sm text-muted-foreground py-4">
          {metricId
            ? 'This metric does not have a segmentation matrix'
            : 'Select a billable metric with matrix dimensions'}
        </p>
      ) : (
        <PricingFields
          key={usageModel}
          pricingType={pricingType}
          control={methods.control}
          currency={currency}
          dimensionHeaders={dimensionHeaders}
          validCombinations={validCombinations}
        />
      )}
    </div>
  )
}

// --- Extra recurring inline ---

const ExtraRecurringInline = ({
  methods,
  currency,
}: {
  methods: Methods<z.ZodType>
  currency: string
}) => (
  <div className="space-y-4">
    <CadenceSelect methods={methods} />
    <PricingFields pricingType="extraRecurring" control={methods.control} currency={currency} />
  </div>
)

