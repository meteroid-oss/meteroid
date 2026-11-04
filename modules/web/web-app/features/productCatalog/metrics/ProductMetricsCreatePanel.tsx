import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query';
import {
  Button,
  FormDescription,
  InputFormField,
  SelectFormField,
  SelectItem,
  TextareaFormField,
} from '@md/ui'
import { D, pipe } from '@mobily/ts-belt'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { AggregationSection } from '@/features/productCatalog/metrics/AggregationSection'
import { EventCodeFormField } from '@/features/productCatalog/metrics/EventCodeFormField'
import { MetricBuilderShell } from '@/features/productCatalog/metrics/MetricBuilderShell'
import { MetricFiltersSection } from '@/features/productCatalog/metrics/MetricFiltersSection'
import { MetricPreview } from '@/features/productCatalog/metrics/MetricPreview'
import { SegmentationMatrixSection } from '@/features/productCatalog/metrics/SegmentationMatrixSection'
import { UnitConversionSection } from '@/features/productCatalog/metrics/UnitConversionSection'
import { UnitLabelSection } from '@/features/productCatalog/metrics/UnitLabelSection'
import { UsageGroupKeySection } from '@/features/productCatalog/metrics/UsageGroupKeySection'
import {
  SettingsFieldGrid,
  SettingsSection,
} from '@/features/settings/components/SettingsSection'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { schemas } from '@/lib/schemas'
import {
  CreateBillableMetricFormData,
  Dimension,
  SimpleSegmentationMatrixFormData,
} from '@/lib/schemas/billableMetrics'
import {
  createBillableMetric,
  getBillableMetric,
  listBillableMetrics,
} from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  Aggregation_AggregationType,
  Aggregation_UnitConversion_UnitConversionRounding,
  MetricFilter_Operator,
} from '@/rpc/api/billablemetrics/v1/models_pb'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

// Shared init-shape builders so the live preview and the create request derive
// the aggregation / segmentation payloads identically from the same form values.
const toAggregationInit = (aggregation: CreateBillableMetricFormData['aggregation']) => ({
  aggregationType: Aggregation_AggregationType[aggregation.aggregationType],
  aggregationKey: aggregation.aggregationKey,
  unitConversion: aggregation.unitConversion && {
    factor: aggregation.unitConversion.factor,
    rounding:
      Aggregation_UnitConversion_UnitConversionRounding[aggregation.unitConversion.rounding],
  },
})

// Form filters → the proto/preview shape (op string → enum). Shared by submit and preview
// so the metered value and the previewed series come from the exact same filter set.
const toFilterInit = (filters?: CreateBillableMetricFormData['filters']) =>
  (filters ?? []).map(f => ({
    property: f.property,
    op: MetricFilter_Operator[f.op],
    values: f.values,
  }))

const toSegmentationMatrixInit = (segmentationMatrix?: SimpleSegmentationMatrixFormData) =>
  segmentationMatrix &&
  (segmentationMatrix.single
    ? { matrix: { case: 'single' as const, value: { dimension: segmentationMatrix.single } } }
    : segmentationMatrix.double
      ? { matrix: { case: 'double' as const, value: segmentationMatrix.double } }
      : segmentationMatrix.linked
        ? {
            matrix: {
              case: 'linked' as const,
              value: {
                ...segmentationMatrix.linked,
                values: pipe(
                  segmentationMatrix.linked.values,
                  D.map(values => ({ values }))
                ),
              },
            },
          }
        : undefined)

interface ProductMetricsCreatePanelProps {
  metricId?: string
  sourceMetricId?: string
}

export const ProductMetricsCreatePanel = ({
  metricId,
  sourceMetricId,
}: ProductMetricsCreatePanelProps) => {
  const queryClient = useQueryClient()
  const [isSubmitting, setIsSubmitting] = useState(false)
  const navigate = useNavigate()
  const showConfirmation = useConfirmationModal()

  const familiesQuery = useQuery(listProductFamilies)
  const families = (familiesQuery.data?.productFamilies ?? []).sort((a, b) =>
    a.id > b.id ? 1 : -1
  )

  // Fetch source metric for duplicate (via create with sourceMetricId)
  const sourceMetricQuery = useQuery(
    getBillableMetric,
    sourceMetricId ? { id: sourceMetricId } : skipToken,
    { enabled: !!sourceMetricId }
  )

  const createBillableMetricMut = useMutation(createBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: listBillableMetrics.parent,
        cardinality: undefined
      }) })
      toast.success('Metric created successfully')
    },
    onError: error => {
      toast.error('Failed to create metric: ' + error.message)
    },
  })

  const methods = useZodForm({
    schema: schemas.billableMetrics.createBillableMetricSchema,
    defaultValues: {
      metricName: '',
      eventCode: '',
      metricDescription: '',
      aggregation: {
        aggregationType: 'COUNT' as const,
      },
      segmentationMatrix: {
        matrixType: 'NONE' as const,
      },
      filters: [],
      unitSingular: '',
      unitPlural: '',
    },
    mode: 'all',
  })

  // Load source metric data when duplicating
  useEffect(() => {
    if (sourceMetricQuery.data?.billableMetric) {
      const metric = sourceMetricQuery.data.billableMetric
      const aggregationTypeKey = Object.keys(Aggregation_AggregationType).find(
        key =>
          Aggregation_AggregationType[key as keyof typeof Aggregation_AggregationType] ===
          metric.aggregation?.aggregationType
      ) as keyof typeof Aggregation_AggregationType | undefined

      const roundingKey =
        metric.aggregation?.unitConversion?.rounding !== undefined
          ? (Object.keys(Aggregation_UnitConversion_UnitConversionRounding).find(
              key =>
                Aggregation_UnitConversion_UnitConversionRounding[
                  key as keyof typeof Aggregation_UnitConversion_UnitConversionRounding
                ] === metric.aggregation?.unitConversion?.rounding
            ) as keyof typeof Aggregation_UnitConversion_UnitConversionRounding | undefined)
          : undefined

      // Convert segmentation matrix from proto format
      let segmentationMatrix: SimpleSegmentationMatrixFormData = { matrixType: 'NONE' }
      if (metric.segmentationMatrix?.matrix) {
        const matrix = metric.segmentationMatrix.matrix
        if (matrix.case === 'single' && matrix.value.dimension) {
          segmentationMatrix = {
            matrixType: 'SINGLE',
            single: matrix.value.dimension as Dimension,
          }
        } else if (matrix.case === 'double') {
          segmentationMatrix = {
            matrixType: 'DOUBLE',
            double: {
              dimension1: matrix.value.dimension1 as Dimension,
              dimension2: matrix.value.dimension2 as Dimension,
            },
          }
        } else if (matrix.case === 'linked') {
          const linkedValues: Record<string, string[]> = {}
          Object.entries(matrix.value.values || {}).forEach(([key, val]) => {
            linkedValues[key] = val.values
          })
          segmentationMatrix = {
            matrixType: 'LINKED',
            linked: {
              dimensionKey: matrix.value.dimensionKey,
              linkedDimensionKey: matrix.value.linkedDimensionKey,
              values: linkedValues as Record<string, [string, ...string[]]>,
            },
          }
        }
      }

      methods.reset({
        metricName: `${metric.name} (Copy)`,
        eventCode: `${metric.code}`,
        metricDescription: metric.description || '',
        productFamilyId: metric.familyLocalId,
        aggregation: {
          aggregationType: aggregationTypeKey || 'COUNT',
          aggregationKey: metric.aggregation?.aggregationKey || undefined,
          unitConversion: metric.aggregation?.unitConversion
            ? {
                factor: metric.aggregation.unitConversion.factor,
                rounding: roundingKey || 'NONE',
              }
            : undefined,
        },
        segmentationMatrix,
        filters: metric.filters.map(f => ({
          property: f.property,
          op: MetricFilter_Operator[f.op] as 'EQUAL' | 'NOT_EQUAL' | 'IN' | 'NOT_IN',
          values: f.values,
        })),
        usageGroupKey: metric.usageGroupKey || undefined,
        unitSingular: metric.unitLabel?.singular ?? '',
        unitPlural: metric.unitLabel?.plural ?? '',
      })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sourceMetricQuery.data])

  useEffect(() => {
    if (families[0]?.localId) {
      methods.setValue('productFamilyId', families[0].localId)
    }
  }, [families])

  const safeClosePanel = () => {
    methods.trigger()
    const isDirty =
      methods.formState.isDirty || Object.keys(methods.formState.dirtyFields).length > 0
    if (isDirty) {
      showConfirmation(
        () => {
          methods.reset()
          navigate('..')
        },
        {
          message:
            'There are unsaved changes. Are you sure you want to close the panel? Your changes will be lost.',
        }
      )
    } else {
      methods.reset()
      navigate('..')
    }
  }

  const submit = useCallback(
    async (input: CreateBillableMetricFormData) => {
      setIsSubmitting(true)
      try {
        // Create new metric (or duplicate via create)
        await createBillableMetricMut.mutateAsync({
          name: input.metricName,
          code: input.eventCode,
          description: input.metricDescription,
          aggregation: toAggregationInit(input.aggregation),
          segmentationMatrix: toSegmentationMatrixInit(input.segmentationMatrix),
          filters: toFilterInit(input.filters),
          usageGroupKey: input.usageGroupKey ?? undefined,
          familyLocalId: input.productFamilyId,
          unitLabel:
            input.unitSingular && input.unitPlural
              ? { singular: input.unitSingular, plural: input.unitPlural }
              : undefined,
        })

        methods.reset()
        navigate('..')
      } catch (error) {
        // Errors are already handled by mutation onError handlers
      } finally {
        setIsSubmitting(false)
      }
    },
    [metricId, createBillableMetricMut, methods, navigate]
  )

  // Live form values driving the inline preview (same helpers as submit).
  const [previewCode, previewAggregation, previewSegmentation, previewGroupKey, previewFilters] =
    methods.watch([
      'eventCode',
      'aggregation',
      'segmentationMatrix',
      'usageGroupKey',
      'filters',
    ])

  const isDuplicating = !!sourceMetricId

  const titles = isDuplicating ? 'Duplicate metric' : 'Register a new metric'

  const buttonTexts = isSubmitting
    ? isDuplicating
      ? 'Creating...'
      : 'Creating...'
    : isDuplicating
      ? 'Create Metric'
      : 'Create Metric'

  const isLoading = isDuplicating && sourceMetricQuery.isLoading

  return (
    <MetricBuilderShell
      title={titles}
      description={
        isDuplicating
          ? 'Create a copy of this metric.'
          : 'Metrics let you aggregate customer usage events into billable units'
      }
      onClose={safeClosePanel}
      onSubmit={methods.handleSubmit(submit)}
      methods={methods}
      formSlot={
        isLoading ? (
          <div className="flex h-40 items-center justify-center text-sm text-muted-foreground">
            Loading metric…
          </div>
        ) : (
          <>
            <SettingsSection
              divided={false}
              title="Basic Information"
              description="Define the core properties of your billable metric"
            >
              <SettingsFieldGrid>
                <InputFormField
                  name="metricName"
                  label="Metric name"
                  control={methods.control}
                  placeholder="Compute (CPU-seconds)"
                />

                {families.length > 1 ? (
                  <SelectFormField
                    name="productFamilyId"
                    label="Product line"
                    layout="vertical"
                    placeholder="Select a product line"
                    empty={families.length === 0}
                    control={methods.control}
                  >
                    {families.map(f => (
                      <SelectItem value={f.localId} key={f.localId}>
                        {f.name}
                      </SelectItem>
                    ))}
                  </SelectFormField>
                ) : (
                  <InputFormField
                    hidden
                    className="hidden"
                    value={families[0]?.localId}
                    control={methods.control}
                    name="productFamilyId"
                  />
                )}

                <div className="space-y-2 sm:col-span-2">
                  <EventCodeFormField
                    name="eventCode"
                    label="Event Code"
                    control={methods.control}
                    placeholder="compute_usage"
                  />
                  <FormDescription>
                    Qualifies an event stream, ex: page_views.
                    <br />A single usage event can be processed by multiple metrics.
                  </FormDescription>
                </div>
                <TextareaFormField
                  name="metricDescription"
                  label="Description"
                  containerClassName="sm:col-span-2"
                  control={methods.control}
                  placeholder="Serverless compute usage for ..."
                />
              </SettingsFieldGrid>
            </SettingsSection>

            <AggregationSection methods={methods} />
            <UnitConversionSection methods={methods} />
            <UnitLabelSection control={methods.control} />
            <UsageGroupKeySection methods={methods} />
            <SegmentationMatrixSection methods={methods} />
            <MetricFiltersSection control={methods.control} />
          </>
        )
      }
      previewSlot={
        isLoading ? null : (
          <MetricPreview
            code={previewCode ?? ''}
            aggregation={
              previewAggregation?.aggregationType
                ? toAggregationInit(previewAggregation)
                : undefined
            }
            segmentationMatrix={toSegmentationMatrixInit(previewSegmentation)}
            filters={toFilterInit(previewFilters)}
            usageGroupKey={previewGroupKey || undefined}
          />
        )
      }
      footerSlot={
        <>
          <Button type="button" variant="outline" onClick={safeClosePanel} disabled={isSubmitting}>
            Cancel
          </Button>
          <Button type="submit" disabled={!methods.formState.isValid || isSubmitting}>
            {buttonTexts}
          </Button>
        </>
      }
    />
  )
}
