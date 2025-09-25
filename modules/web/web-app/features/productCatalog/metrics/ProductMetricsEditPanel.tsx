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
import { useCallback, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { AccordionPanel } from '@/components/AccordionPanel'
import { AggregationSection } from '@/features/productCatalog/metrics/AggregationSection'
import { SegmentationMatrixContent } from '@/features/productCatalog/metrics/SegmentationMatrixSection'
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

  const createBillableMetricMut = useMutation(createBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listBillableMetrics.service.typeName] })
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

  useEffect(() => {
    console.log('errors', JSON.stringify(methods.formState.errors))
  }, [methods.formState.errors])

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
      const res = await createBillableMetricMut.mutateAsync({
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

      res.billableMetric?.id && toast.success('Metric created')
      methods.reset()
      navigate('..')
    },
    [methods, navigate]
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
                <div className="px-2 relative">
                  <section className="mb-2 space-y-6 ">
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
                      className="max-w-sm "
                      control={methods.control}
                      placeholder="Serverless compute usage for ..."
                    />
                  </section>
                  <Separator />
                  <AggregationSection methods={methods} />
                  <Separator />
                  <UnitConversionSection methods={methods} />
                  <Separator />

                  <AccordionPanel
                    title={
                      <div className="space-x-4 items-center flex pr-4">
                        <h3>Segmentation</h3>
                        <span className="text-xs text-muted-foreground">optional</span>
                      </div>
                    }
                    defaultOpen={false}
                  >
                    <div className="space-y-8">
                      <FormDescription>
                        <p>
                          Control how usage data is organized and aggregated for pricing and analytics.
                        </p>
                      </FormDescription>

                      {/* Matrix Segmentation */}
                      <div className="space-y-4">
                        <div>
                          <h4 className="text-sm font-medium">Matrix</h4>
                          <p className="text-sm text-muted-foreground mt-1">
                            Define different pricing based on one or two dimensions with fixed values.
                            <br />
                            For example, different pricing per cloud provider and region.
                          </p>
                        </div>
                        <div className="pl-4 border-l-2 border-border">
                          <SegmentationMatrixContent methods={methods} />
                        </div>
                      </div>

                      <Separator />

                      {/* Group Key */}
                      <div className="space-y-4">
                        <div>
                          <h4 className="text-sm font-medium">Group Key</h4>
                          <p className="text-sm text-muted-foreground mt-1">
                            Group usage data by a dynamic dimension for detailed analytics.
                            <br />
                            For example, separate usage by cluster, tenant, or workspace.
                          </p>
                          <p className="text-xs text-muted-foreground font-medium mt-2">
                            This does not impact pricing, only data organization.
                          </p>
                        </div>
                        <div className="pl-4 border-l-2 border-border">
                          <InputFormField
                            name="usageGroupKey"
                            label="Group key"
                            control={methods.control}
                            placeholder="cluster_id"
                            className="max-w-xs"
                          />
                        </div>
                      </div>
                    </div>
                  </AccordionPanel>
                </div>
              </ScrollArea>
              <Separator />
              <SheetFooter className="pt-3">
                <Button disabled={!methods.formState.isValid} type="submit">
                  Create
                </Button>
              </SheetFooter>
            </form>
          </Form>
        </SheetContent>
      </Sheet>
    </>
  )
}
