import { useMutation } from '@connectrpc/connect-query'
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
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { schemas } from '@/lib/schemas'
import { CreateBillableMetricFormData } from '@/lib/schemas/billableMetrics'
import {
  createBillableMetric,
  listBillableMetrics,
} from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  Aggregation_AggregationType,
  Aggregation_UnitConversion_UnitConversionRounding,
} from '@/rpc/api/billablemetrics/v1/models_pb'
import { listProductFamilies } from '@/rpc/api/productfamilies/v1/productfamilies-ProductFamiliesService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

export const ProductMetricsEditPanel = () => {
  const queryClient = useQueryClient()
  const [isSubmitting, setIsSubmitting] = useState(false)

  const createBillableMetricMut = useMutation(createBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listBillableMetrics.service.typeName] })
      toast.success('Metric created successfully')
    },
    onError: error => {
      toast.error('Failed to create metric: ' + error.message)
    },
  })

  const familiesQuery = useQuery(listProductFamilies)
  const families = (familiesQuery.data?.productFamilies ?? []).sort((a, b) =>
    a.id > b.id ? 1 : -1
  )

  const navigate = useNavigate()
  const showConfirmation = useConfirmationModal()

  const methods = useZodForm({
    schema: schemas.billableMetrics.createBillableMetricSchema,
    defaultValues: {
      metricName: '',
      eventCode: '',
      metricDescription: '',
      aggregation: {
        aggregationType: 'COUNT',
      },
      segmentationMatrix: {
        matrixType: 'NONE',
      },
    },
    mode: 'all',
  })

  const safeClosePanel = () => {
    methods.trigger()
    console.log('dbg', methods.formState.isValid)
    console.log('dbg2', JSON.stringify(methods.formState))

    console.log(
      'dbg',
      methods.formState.errors,
      methods.formState.isValid,
      methods.formState.isDirty,
      Object.keys(methods.formState.dirtyFields).length
    )

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
        console.error('Failed to create metric:', error)
      } finally {
        setIsSubmitting(false)
      }
    },
    [createBillableMetricMut, methods, navigate]
  )

  useEffect(() => {
    methods.setValue('productFamilyId', families[0]?.localId)
  }, [families])

  // TODO try without the form, with onConfirm
  return (
    <>
      <Sheet open={true} onOpenChange={safeClosePanel}>
        <SheetContent size="medium">
          <Form {...methods}>
            <form onSubmit={methods.handleSubmit(submit)} className="relative h-full flex flex-col">
              <SheetHeader className="border-b border-border pb-3 mb-3">
                <SheetTitle>Register a new metric</SheetTitle>
                <SheetDescription>
                  Metrics let you aggregate customer usage events into billable units
                </SheetDescription>
              </SheetHeader>
              <ScrollArea className="flex grow pr-2 -mr-4">
                <div className="px-2 relative space-y-6">
                  {/* Basic Information */}
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
                  <SegmentationMatrixSection methods={methods} />
                </div>
              </ScrollArea>
              <Separator />
              <SheetFooter className="pt-3 space-x-3">
                <Button variant="outline" onClick={safeClosePanel} disabled={isSubmitting}>
                  Cancel
                </Button>
                <Button type="submit" disabled={!methods.formState.isValid || isSubmitting}>
                  {isSubmitting ? 'Creating...' : 'Create Metric'}
                </Button>
              </SheetFooter>
            </form>
          </Form>
        </SheetContent>
      </Sheet>
    </>
  )
}
