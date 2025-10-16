import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Form,
  GenericFormField,
  Input,
  InputFormField,
  Label,
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
import { useQueryClient } from '@tanstack/react-query'
import { PlusIcon, XIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'
import { z } from 'zod'

import { aggregationTypeMapper } from '@/features/productCatalog/metrics/ProductMetricsTable'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  getBillableMetric,
  listBillableMetrics,
  updateBillableMetric,
} from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  Aggregation_AggregationType,
  Aggregation_UnitConversion_UnitConversionRounding,
  SegmentationMatrixValuesUpdate,
  SegmentationMatrixValuesUpdate_DoubleDimensionValues,
  SegmentationMatrixValuesUpdate_LinkedDimensionValues,
  SegmentationMatrixValuesUpdate_LinkedDimensionValues_DimensionValues,
  SegmentationMatrixValuesUpdate_SingleDimensionValues,
} from '@/rpc/api/billablemetrics/v1/models_pb'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

interface ProductMetricsEditViewProps {
  metricId: string
}

// Schema for editable fields only
const editMetricSchema = z.object({
  name: z.string().min(3, 'Name must be at least 3 characters'),
  description: z.string().optional(),
  unitConversionFactor: z.coerce.number().positive().optional(),
  unitConversionRounding: z.enum(['NONE', 'UP', 'DOWN', 'NEAREST']).optional(),
  // For segmentation, we only allow editing VALUES, not keys or structure
  singleDimensionValues: z.array(z.string()).optional(),
  doubleDimension1Values: z.array(z.string()).optional(),
  doubleDimension2Values: z.array(z.string()).optional(),
  linkedDimensionValues: z.record(z.string(), z.array(z.string())).optional(),
})

type EditMetricFormData = z.infer<typeof editMetricSchema>

// Helper component for managing string arrays
function StringArrayEditor({
  label,
  value = [],
  onChange,
}: {
  label: string
  value?: string[]
  onChange: (value: string[]) => void
}) {
  const handleAdd = () => {
    onChange([...value, ''])
  }

  const handleRemove = (index: number) => {
    onChange(value.filter((_, i) => i !== index))
  }

  const handleChange = (index: number, newValue: string) => {
    const updated = [...value]
    updated[index] = newValue
    onChange(updated)
  }

  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <div className="space-y-2">
        {value.map((item, idx) => (
          <div key={idx} className="flex items-center gap-2">
            <Input
              value={item}
              onChange={e => handleChange(idx, e.target.value)}
              placeholder="Enter value..."
              className="flex-1"
            />
            <Button
              type="button"
              variant="ghost"
              size="icon"
              onMouseDown={e => {
                e.preventDefault()
                handleRemove(idx)
              }}
            >
              <XIcon className="h-4 w-4" />
            </Button>
          </div>
        ))}
        <Button
          type="button"
          variant="outline"
          size="sm"
          onMouseDown={e => {
            e.preventDefault()
            handleAdd()
          }}
          className="w-full"
        >
          <PlusIcon className="h-4 w-4 mr-2" />
          Add Value
        </Button>
      </div>
    </div>
  )
}

export const ProductMetricsEditView = ({ metricId }: ProductMetricsEditViewProps) => {
  const queryClient = useQueryClient()
  const [isSubmitting, setIsSubmitting] = useState(false)
  const navigate = useNavigate()
  const showConfirmation = useConfirmationModal()

  const metricQuery = useQuery(getBillableMetric, { id: metricId })
  const metric = metricQuery.data?.billableMetric

  // Track segmentation matrix type (immutable, just for display)
  const [matrixType, setMatrixType] = useState<'NONE' | 'SINGLE' | 'DOUBLE' | 'LINKED'>('NONE')
  const [dimensionKeys, setDimensionKeys] = useState<{
    single?: string
    double1?: string
    double2?: string
    linkedKey?: string
    linkedLinkedKey?: string
  }>({})

  const updateBillableMetricMut = useMutation(updateBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listBillableMetrics.service.typeName] })
      await queryClient.invalidateQueries({ queryKey: [getBillableMetric.service.typeName] })
      toast.success('Metric updated successfully')
    },
    onError: error => {
      toast.error('Failed to update metric: ' + error.message)
    },
  })

  const methods = useZodForm({
    schema: editMetricSchema,
    defaultValues: {
      name: '',
      description: '',
      unitConversionRounding: 'NONE',
    },
  })

  // Load metric data
  useEffect(() => {
    if (metric) {
      const matrix = metric.segmentationMatrix?.matrix
      const aggregationType = metric.aggregation?.aggregationType

      // Prepare dimension values
      let singleValues: string[] | undefined
      let double1Values: string[] | undefined
      let double2Values: string[] | undefined
      let linkedValues: Record<string, string[]> | undefined

      // Determine matrix type and extract keys/values
      if (matrix?.case === 'single' && matrix.value.dimension) {
        setMatrixType('SINGLE')
        setDimensionKeys({ single: matrix.value.dimension.key })
        singleValues = matrix.value.dimension.values || []
      } else if (matrix?.case === 'double') {
        setMatrixType('DOUBLE')
        setDimensionKeys({
          double1: matrix.value.dimension1?.key,
          double2: matrix.value.dimension2?.key,
        })
        double1Values = matrix.value.dimension1?.values || []
        double2Values = matrix.value.dimension2?.values || []
      } else if (matrix?.case === 'linked') {
        setMatrixType('LINKED')
        setDimensionKeys({
          linkedKey: matrix.value.dimensionKey,
          linkedLinkedKey: matrix.value.linkedDimensionKey,
        })
        linkedValues = {}
        Object.entries(matrix.value.values || {}).forEach(([key, val]) => {
          linkedValues![key] = val.values || []
        })
      } else {
        setMatrixType('NONE')
      }

      // Unit conversion should be hidden/reset for COUNT and COUNT_DISTINCT
      const showUnitConversion =
        aggregationType !== Aggregation_AggregationType.COUNT &&
        aggregationType !== Aggregation_AggregationType.COUNT_DISTINCT

      const roundingKey =
        showUnitConversion && metric.aggregation?.unitConversion?.rounding !== undefined
          ? (Object.keys(Aggregation_UnitConversion_UnitConversionRounding).find(
              key =>
                Aggregation_UnitConversion_UnitConversionRounding[
                  key as keyof typeof Aggregation_UnitConversion_UnitConversionRounding
                ] === metric.aggregation?.unitConversion?.rounding
            ) as keyof typeof Aggregation_UnitConversion_UnitConversionRounding | undefined)
          : undefined

      // Reset with ALL values including dimension values
      methods.reset({
        name: metric.name,
        description: metric.description || '',
        unitConversionFactor: showUnitConversion
          ? metric.aggregation?.unitConversion?.factor
          : undefined,
        unitConversionRounding: showUnitConversion ? roundingKey || 'NONE' : 'NONE',
        singleDimensionValues: singleValues,
        doubleDimension1Values: double1Values,
        doubleDimension2Values: double2Values,
        linkedDimensionValues: linkedValues,
      })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [metric])

  const safeClosePanel = () => {
    const isDirty = methods.formState.isDirty
    if (isDirty) {
      showConfirmation(
        () => {
          methods.reset()
          navigate('..')
        },
        {
          message:
            'There are unsaved changes. Are you sure you want to close? Your changes will be lost.',
        }
      )
    } else {
      methods.reset()
      navigate('..')
    }
  }

  const handleSubmit = async (data: EditMetricFormData) => {
    setIsSubmitting(true)
    try {
      const filterEmpty = (arr?: string[]) => (arr || []).filter(s => s.trim() !== '')

      let segmentationMatrixValues: SegmentationMatrixValuesUpdate | undefined = undefined

      if (matrixType === 'SINGLE') {
        segmentationMatrixValues = new SegmentationMatrixValuesUpdate({
          values: {
            case: 'single',
            value: new SegmentationMatrixValuesUpdate_SingleDimensionValues({
              values: filterEmpty(data.singleDimensionValues),
            }),
          },
        })
      } else if (matrixType === 'DOUBLE') {
        segmentationMatrixValues = new SegmentationMatrixValuesUpdate({
          values: {
            case: 'double',
            value: new SegmentationMatrixValuesUpdate_DoubleDimensionValues({
              dimension1Values: filterEmpty(data.doubleDimension1Values),
              dimension2Values: filterEmpty(data.doubleDimension2Values),
            }),
          },
        })
      } else if (matrixType === 'LINKED') {
        const linkedValues: {
          [key: string]: SegmentationMatrixValuesUpdate_LinkedDimensionValues_DimensionValues
        } = {}
        Object.entries(data.linkedDimensionValues || {}).forEach(([key, vals]) => {
          linkedValues[key] =
            new SegmentationMatrixValuesUpdate_LinkedDimensionValues_DimensionValues({
              values: filterEmpty(vals),
            })
        })

        segmentationMatrixValues = new SegmentationMatrixValuesUpdate({
          values: {
            case: 'linked',
            value: new SegmentationMatrixValuesUpdate_LinkedDimensionValues({
              values: linkedValues,
            }),
          },
        })
      }

      await updateBillableMetricMut.mutateAsync({
        id: metricId,
        name: data.name,
        description: data.description || undefined,
        unitConversion: data.unitConversionFactor
          ? {
              factor: data.unitConversionFactor,
              rounding:
                Aggregation_UnitConversion_UnitConversionRounding[
                  data.unitConversionRounding || 'NONE'
                ],
            }
          : undefined,
        segmentationMatrixValues,
      })

      methods.reset()
      navigate('..')
    } catch (error) {
      // Error handled by mutation
    } finally {
      setIsSubmitting(false)
    }
  }

  const isLoading = metricQuery.isLoading

  return (
    <Sheet open={true} onOpenChange={safeClosePanel}>
      <SheetContent size="medium">
        {isLoading ? (
          <div className="flex items-center justify-center h-full">
            <p>Loading metric...</p>
          </div>
        ) : (
          <Form {...methods}>
            <form
              onSubmit={methods.handleSubmit(handleSubmit)}
              className="relative h-full flex flex-col"
            >
              <SheetHeader className="border-b border-border pb-3 mb-3">
                <SheetTitle>Edit Metric</SheetTitle>
                <SheetDescription>
                  Only some properties can be edited. Aggregation dimensions and event settings are
                  immutable.
                </SheetDescription>
              </SheetHeader>
              <ScrollArea className="flex grow pr-2 -mr-4">
                <div className="px-2 relative space-y-6 pb-4">
                  {/* Immutable Info Section */}
                  <div className="space-y-3 p-4 bg-muted/30 rounded-lg border border-border">
                    <div className="grid grid-cols-3 gap-3 text-sm">
                      <div>
                        <div className="text-muted-foreground text-xs">Event Code</div>
                        <div className="font-mono">{metric?.code}</div>
                      </div>
                      <div>
                        <div className="text-muted-foreground text-xs">Aggregation</div>
                        <div className="font-mono">
                          {metric?.aggregation?.aggregationType &&
                            aggregationTypeMapper[metric.aggregation.aggregationType]}
                          {metric?.aggregation?.aggregationKey &&
                            ` (${metric.aggregation.aggregationKey})`}
                        </div>
                      </div>
                      {metric?.usageGroupKey && (
                        <div>
                          <div className="text-muted-foreground text-xs">Usage Group Key</div>
                          <div className="font-mono">{metric.usageGroupKey}</div>
                        </div>
                      )}
                    </div>
                  </div>

                  {/* Editable Fields */}
                  <div className="space-y-4">
                    <h3 className="text-sm font-medium">Details</h3>

                    <InputFormField
                      name="name"
                      label="Metric Name"
                      control={methods.control}
                      placeholder="Compute (CPU-seconds)"
                    />

                    <TextareaFormField
                      name="description"
                      label="Description"
                      control={methods.control}
                      placeholder="Serverless compute usage for ..."
                    />
                  </div>

                  {/* Unit Conversion - hidden for COUNT and COUNT_DISTINCT */}
                  {metric?.aggregation?.aggregationType !== Aggregation_AggregationType.COUNT &&
                    metric?.aggregation?.aggregationType !==
                      Aggregation_AggregationType.COUNT_DISTINCT && (
                      <>
                        <Separator />
                        <div className="space-y-4">
                          <div>
                            <h3 className="text-sm font-medium">Unit Conversion</h3>
                            <p className="text-xs text-muted-foreground mt-1">
                              Optional conversion factor and rounding for the aggregated value
                            </p>
                          </div>

                          <div className="grid grid-cols-2 gap-4">
                            <InputFormField
                              name="unitConversionFactor"
                              label="Factor"
                              control={methods.control}
                              type="number"
                              placeholder="1000"
                            />
                            <SelectFormField
                              name="unitConversionRounding"
                              label="Rounding"
                              control={methods.control}
                              placeholder="Select rounding mode"
                            >
                              <SelectItem value="NONE">None</SelectItem>
                              <SelectItem value="UP">Up</SelectItem>
                              <SelectItem value="DOWN">Down</SelectItem>
                              <SelectItem value="NEAREST">Nearest</SelectItem>
                            </SelectFormField>
                          </div>
                        </div>
                      </>
                    )}

                  {/* Segmentation Matrix Values */}
                  {matrixType !== 'NONE' && (
                    <>
                      <Separator />
                      <div className="space-y-4 ">
                        <div>
                          <div className="flex items-center gap-2">
                            <h3 className="text-sm font-medium">Segmentation Values</h3>
                            <Badge variant="secondary" className="text-xs">
                              {matrixType}
                            </Badge>
                          </div>
                          <p className="text-xs text-muted-foreground mt-1">
                            Dimension keys are immutable. Only values can be edited.
                          </p>
                        </div>

                        {matrixType === 'SINGLE' && (
                          <div className="space-y-3">
                            <div className="text-sm">
                              <span className="text-muted-foreground">Dimension: </span>
                              <span className="font-mono">{dimensionKeys.single}</span>
                            </div>
                            <GenericFormField
                              control={methods.control}
                              name="singleDimensionValues"
                              render={({ field }) => (
                                <StringArrayEditor
                                  label="Allowed Values"
                                  value={field.value}
                                  onChange={field.onChange}
                                />
                              )}
                            />
                          </div>
                        )}

                        {matrixType === 'DOUBLE' && (
                          <div className="space-y-4">
                            <div className="space-y-3">
                              <div className="text-sm">
                                <span className="text-muted-foreground">Dimension 1: </span>
                                <span className="font-mono">{dimensionKeys.double1}</span>
                              </div>
                              <GenericFormField
                                control={methods.control}
                                name="doubleDimension1Values"
                                render={({ field }) => (
                                  <StringArrayEditor
                                    label="Allowed Values"
                                    value={field.value}
                                    onChange={field.onChange}
                                  />
                                )}
                              />
                            </div>
                            <div className="space-y-3">
                              <div className="text-sm">
                                <span className="text-muted-foreground">Dimension 2: </span>
                                <span className="font-mono">{dimensionKeys.double2}</span>
                              </div>
                              <GenericFormField
                                control={methods.control}
                                name="doubleDimension2Values"
                                render={({ field }) => (
                                  <StringArrayEditor
                                    label="Allowed Values"
                                    value={field.value}
                                    onChange={field.onChange}
                                  />
                                )}
                              />
                            </div>
                          </div>
                        )}

                        {matrixType === 'LINKED' && (
                          <div className="space-y-3">
                            <div className="text-sm space-y-1">
                              <div>
                                <span className="text-muted-foreground">Primary: </span>
                                <span className="font-mono">{dimensionKeys.linkedKey}</span>
                              </div>
                              <div>
                                <span className="text-muted-foreground">Linked: </span>
                                <span className="font-mono">{dimensionKeys.linkedLinkedKey}</span>
                              </div>
                            </div>
                            <div className="text-xs text-muted-foreground p-2 bg-muted/30 rounded">
                              Linked dimension editing is complex. Use the duplicate feature to
                              create a new metric with different linked values.
                            </div>
                          </div>
                        )}
                      </div>
                    </>
                  )}
                </div>
              </ScrollArea>
              <Separator />
              <SheetFooter className="pt-3 space-x-3">
                <Button variant="outline" onClick={safeClosePanel} disabled={isSubmitting}>
                  Cancel
                </Button>
                <Button type="submit" disabled={!methods.formState.isValid || isSubmitting}>
                  {isSubmitting ? 'Saving...' : 'Save Changes'}
                </Button>
              </SheetFooter>
            </form>
          </Form>
        )}
      </SheetContent>
    </Sheet>
  )
}
