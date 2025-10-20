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
import { useEffect, useRef, useState } from 'react'
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
  linkedDimensionValues: z
    .array(
      z.object({
        key: z.string().min(1, 'Key cannot be empty'),
        values: z.array(z.string()),
      })
    )
    .optional()
    .refine(
      val => {
        if (!val) return true
        const keys = val.map(v => v.key.trim())
        // No duplicate keys
        return new Set(keys).size === keys.length
      },
      { message: 'All dimension keys must be unique' }
    ),
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

function LinkedDimensionsEditor({
  value = [],
  onChange,
  primaryKey,
  linkedKey,
}: {
  value?: Array<{ key: string; values: string[] }>
  onChange: (value: Array<{ key: string; values: string[] }>) => void
  primaryKey?: string
  linkedKey?: string
}) {
  // Store raw input strings to preserve commas during typing
  const [entries, setEntries] = useState<Array<{ id: string; key: string; valuesInput: string }>>(
    () => {
      return value.map(({ key, values }) => ({
        id: Math.random().toString(36).substring(2),
        key,
        valuesInput: values.join(','),
      }))
    }
  )

  const isOurChangeRef = useRef(false)

  useEffect(() => {
    if (isOurChangeRef.current) {
      isOurChangeRef.current = false
      return
    }

    setEntries(
      value.map(({ key, values }) => ({
        id: Math.random().toString(36).substring(2),
        key,
        valuesInput: values.join(','),
      }))
    )
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value])

  const notifyChange = (newEntries: Array<{ id: string; key: string; valuesInput: string }>) => {
    const updated = newEntries.map(({ key, valuesInput }) => ({
      key: key.trim(),
      values: valuesInput
        .split(',')
        .map(v => v.trim())
        .filter(Boolean),
    }))
    isOurChangeRef.current = true
    onChange(updated)
  }

  const handleAddKey = () => {
    const newEntries = [
      ...entries,
      { id: Math.random().toString(36).substring(2), key: '', valuesInput: '' },
    ]
    setEntries(newEntries)
    notifyChange(newEntries)
  }

  const handleRemoveKey = (id: string) => {
    const newEntries = entries.filter(e => e.id !== id)
    setEntries(newEntries)
    notifyChange(newEntries)
  }

  const handleKeyChange = (id: string, newKey: string) => {
    const newEntries = entries.map(e => (e.id === id ? { ...e, key: newKey } : e))
    setEntries(newEntries)
    notifyChange(newEntries)
  }

  const handleValuesInputChange = (id: string, valuesInput: string) => {
    const newEntries = entries.map(e => (e.id === id ? { ...e, valuesInput } : e))
    setEntries(newEntries)
    notifyChange(newEntries)
  }

  const hasEmptyKeys = entries.some(e => !e.key.trim())
  const hasDuplicates = entries.some(
    (e, i) => e.key.trim() && entries.findIndex(e2 => e2.key.trim() === e.key.trim()) !== i
  )

  return (
    <div className="space-y-4">
      {entries.length === 0 && (
        <div className="text-sm text-muted-foreground p-4 border border-dashed rounded-lg text-center">
          No mappings defined. Add one below.
        </div>
      )}

      {entries.map(({ id, key, valuesInput }, index) => {
        const isDuplicate =
          key.trim() && entries.findIndex(e => e.key.trim() === key.trim()) !== index
        const isEmpty = !key.trim()

        return (
          <div key={id} className="space-y-1">
            <div className="flex gap-2 items-start">
              <div className="flex-1">
                <Input
                  value={key}
                  onChange={e => handleKeyChange(id, e.target.value)}
                  placeholder={`${primaryKey || 'Primary'} (e.g. AWS)`}
                />
                {isEmpty && key !== '' && (
                  <p className="text-xs text-destructive mt-1">Key cannot be empty</p>
                )}
                {isDuplicate && <p className="text-xs text-destructive mt-1">Duplicate key</p>}
              </div>
              <div className="flex-[2]">
                <Input
                  value={valuesInput}
                  onChange={e => handleValuesInputChange(id, e.target.value)}
                  placeholder={`${linkedKey || 'Linked'} (e.g. us-east-1, us-west-2)`}
                />
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onMouseDown={e => {
                  e.preventDefault()
                  handleRemoveKey(id)
                }}
              >
                Remove
              </Button>
            </div>
          </div>
        )
      })}

      <Button
        type="button"
        variant="outline"
        size="sm"
        onMouseDown={e => {
          e.preventDefault()
          handleAddKey()
        }}
        className="w-full mt-2"
      >
        <PlusIcon className="h-4 w-4 mr-2" />
        Add Mapping
      </Button>

      {(hasEmptyKeys || hasDuplicates) && (
        <p className="text-xs text-destructive mt-2">Fix validation errors before saving</p>
      )}
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
      let linkedValues: Array<{ key: string; values: string[] }> | undefined

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

        linkedValues = Object.entries(matrix.value.values || {}).map(([key, val]) => ({
          key,
          values: val.values || [],
        }))
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
        ;(data.linkedDimensionValues || []).forEach(({ key, values }) => {
          if (key.trim()) {
            linkedValues[key.trim()] =
              new SegmentationMatrixValuesUpdate_LinkedDimensionValues_DimensionValues({
                values: filterEmpty(values),
              })
          }
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
              onKeyDown={e => {
                if (e.key === 'Enter' && e.target instanceof HTMLInputElement) {
                  e.preventDefault()
                }
              }}
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
                            <GenericFormField
                              control={methods.control}
                              name="linkedDimensionValues"
                              render={({ field }) => (
                                <>
                                  <LinkedDimensionsEditor
                                    value={field.value}
                                    onChange={field.onChange}
                                    primaryKey={dimensionKeys.linkedKey}
                                    linkedKey={dimensionKeys.linkedLinkedKey}
                                  />
                                </>
                              )}
                            />
                          </div>
                        )}
                      </div>
                    </>
                  )}
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
