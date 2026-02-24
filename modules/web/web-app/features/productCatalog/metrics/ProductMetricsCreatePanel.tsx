import { disableQuery, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Form,
  FormDescription,
  InputFormField,
  ScrollArea,
  SelectFormField,
  SelectItem,
  Separator,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  TextareaFormField,
} from '@md/ui'
import { D, pipe } from '@mobily/ts-belt'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { AggregationSection } from '@/features/productCatalog/metrics/AggregationSection'
import { SegmentationMatrixSection } from '@/features/productCatalog/metrics/SegmentationMatrixSection'
import { UnitConversionSection } from '@/features/productCatalog/metrics/UnitConversionSection'
import { UsageGroupKeySection } from '@/features/productCatalog/metrics/UsageGroupKeySection'
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
} from '@/rpc/api/billablemetrics/v1/models_pb'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

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
    sourceMetricId ? { id: sourceMetricId } : disableQuery,
    { enabled: !!sourceMetricId }
  )

  const createBillableMetricMut = useMutation(createBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listBillableMetrics.service.typeName] })
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
        productFamilyId: metric.localId,
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
        usageGroupKey: metric.usageGroupKey || undefined,
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
          aggregation: {
            aggregationType: Aggregation_AggregationType[input.aggregation.aggregationType],
            aggregationKey: input.aggregation.aggregationKey,
            unitConversion: input.aggregation.unitConversion && {
              factor: input.aggregation.unitConversion.factor,
              rounding:
                Aggregation_UnitConversion_UnitConversionRounding[
                  input.aggregation.unitConversion.rounding
                ],
            },
          },
          segmentationMatrix: {
            matrix:
              input.segmentationMatrix &&
              (input.segmentationMatrix.single
                ? {
                    case: 'single',
                    value: {
                      dimension: input.segmentationMatrix.single,
                    },
                  }
                : input.segmentationMatrix.double
                  ? {
                      case: 'double',
                      value: input.segmentationMatrix.double,
                    }
                  : input.segmentationMatrix.linked
                    ? {
                        case: 'linked',
                        value: {
                          ...input.segmentationMatrix.linked,
                          values: pipe(
                            input.segmentationMatrix.linked.values,
                            D.map(values => ({ values }))
                          ),
                        },
                      }
                    : undefined),
          },
          usageGroupKey: input.usageGroupKey ?? undefined,
          familyLocalId: input.productFamilyId,
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
    <>
      <Sheet open={true} onOpenChange={safeClosePanel}>
        <SheetContent size="medium">
          {isLoading ? (
            <div className="flex items-center justify-center h-full">
              <p>Loading metric...</p>
            </div>
          ) : (
            <Form {...methods}>
              <form
                onSubmit={methods.handleSubmit(submit)}
                onKeyDown={e => {
                  if (e.key === 'Enter' && e.target instanceof HTMLInputElement) {
                    e.preventDefault()
                  }
                }}
                className="relative h-full flex flex-col"
              >
                <SheetHeader className="border-b border-border pb-3 mb-3">
                  <SheetTitle>{titles}</SheetTitle>
                  <SheetDescription>
                    {isDuplicating
                      ? 'Create a copy of this metric.'
                      : 'Metrics let you aggregate customer usage events into billable units'}
                  </SheetDescription>
                </SheetHeader>
                <ScrollArea className="flex grow pr-2 -mr-4">
                  <div className="px-2 relative space-y-6">
                    <>
                      {/* Create/Duplicate Mode - Full Form */}
                      <div className="space-y-4">
                        <div className="space-y-1">
                          <h3 className="text-sm font-medium">Basic Information</h3>
                          <p className="text-xs text-muted-foreground">
                            Define the core properties of your billable metric
                          </p>
                        </div>

                        <div className="space-y-4 pl-4 border-l-2 border-muted">
                          <InputFormField
                            name="metricName"
                            label="Metric name"
                            control={methods.control}
                            placeholder="Compute (CPU-seconds)"
                            className="max-w-sm"
                          />

                          {families.length > 1 ? (
                            <SelectFormField
                              name="productFamilyId"
                              label="Product line"
                              layout="vertical"
                              placeholder="Select a product line"
                              className="max-w-sm"
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

                          <div className="space-y-2">
                            <InputFormField
                              name="eventCode"
                              label="Event Code"
                              control={methods.control}
                              placeholder="compute_usage"
                              className="max-w-sm"
                            />
                            <FormDescription>
                              Qualifies an event stream, ex: page_views.
                              <br />A single usage event can be processed by multiple metrics !
                            </FormDescription>
                          </div>
                          <TextareaFormField
                            name="metricDescription"
                            label="Description"
                            className="max-w-sm"
                            control={methods.control}
                            placeholder="Serverless compute usage for ..."
                          />
                        </div>
                      </div>

                      {/* Configuration Sections */}
                      <AggregationSection methods={methods} />
                      <UnitConversionSection methods={methods} />
                      <Separator />
                      <UsageGroupKeySection methods={methods} />
                      <SegmentationMatrixSection methods={methods} />
                    </>
                  </div>
                </ScrollArea>
                <Separator />
                <SheetFooter className="pt-3 space-x-3">
                  <Button
                    type="button"
                    variant="outline"
                    onClick={safeClosePanel}
                    disabled={isSubmitting}
                  >
                    Cancel
                  </Button>
                  <Button type="submit" disabled={!methods.formState.isValid || isSubmitting}>
                    {buttonTexts}
                  </Button>
                </SheetFooter>
              </form>
            </Form>
          )}
        </SheetContent>
      </Sheet>
    </>
  )
}
