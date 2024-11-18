import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import {
  Button,
  Form,
  FormDescription,
  InputFormField,
  ScrollArea,
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
import { Flex } from '@ui/components/legacy'
import { useCallback, useEffect } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { AccordionPanel } from '@/components/AccordionPanel'
import { AggregationSection } from '@/features/productCatalog/metrics/AggregationSection'
import { SegmentationMatrixSection } from '@/features/productCatalog/metrics/SegmentationMatrixSection'
import { UnitConversionSection } from '@/features/productCatalog/metrics/UnitConversionSection'
import { useZodForm } from '@/hooks/useZodForm'
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
import { useTypedParams } from '@/utils/params'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

// TODO https://doc.getlago.com/docs/guide/billable-metrics/dimensions
// Add Dimension matrix ()
// One is fixed => allow custom pricing
// The other is dynamic (group key) => only for invoice
export const ProductMetricsEditPanel = () => {
  const queryClient = useQueryClient()

  const createBillableMetricMut = useMutation(createBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listBillableMetrics) })
    },
  })
  const { familyLocalId } = useTypedParams<{ familyLocalId: string }>()

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
    mode: 'onBlur',
  })

  useEffect(() => {
    console.log('errors', methods.formState.errors)
  }, [methods.formState.errors])

  const safeClosePanel = () => {
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
        familyLocalId,
      })

      res.billableMetric?.id && toast.success('Metric created')
      methods.reset()
      navigate('..')
    },
    [methods, navigate]
  )

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
                  <section className="">
                    <Flex direction="column" gap={spaces.space7}>
                      <InputFormField
                        name="metricName"
                        label="Metric name"
                        control={methods.control}
                        placeholder="Compute (GB-hr)"
                        className="max-w-xs"
                      />

                      <div className="space-y-2">
                        <InputFormField
                          name="eventCode"
                          label="Event Code"
                          control={methods.control}
                          placeholder="compute_usage"
                          className="max-w-xs"
                        />
                        <FormDescription>
                          Qualifies an event stream, ex: page_views.
                          <br />A single usage event can be processed by multiple metrics !
                        </FormDescription>
                      </div>
                      <TextareaFormField
                        name="metricDescription"
                        label="Description"
                        className="max-w-lg"
                        control={methods.control}
                        placeholder="Serverless compute usage for ..."
                      />
                    </Flex>
                  </section>
                  <Separator />
                  <AggregationSection methods={methods} />
                  <Separator />
                  <UnitConversionSection methods={methods} />
                  <Separator />
                  <SegmentationMatrixSection methods={methods} />

                  <Separator />

                  <AccordionPanel
                    title={
                      <div className="space-x-4 items-center flex pr-4">
                        <h3>Analytics Grouping Key</h3>
                        <span className="text-xs text-muted-foreground">optional</span>
                      </div>
                    }
                    defaultOpen={false}
                  >
                    <div className="space-y-6">
                      <FormDescription>
                        <p>
                          Specify a dimension to group items by in the API endpoints.
                          <br />
                          For example, a tenant, a workspace or a cluster identifier can be used.
                        </p>
                        <p className="font-bold">This does not impact pricing.</p>

                        {/* <p>TODO how does tier pricing work with this ?</p>
                  <p>
                    TODO : should we allow dynamic grouping for billing as well ? to have tiered
                    pricing per tenant for example. Or should it be separate plans ? (ex: cloudflare
                    sites, chargebee sites)
                  </p> */}
                      </FormDescription>
                      <div>
                        <InputFormField
                          name="usageGroupKey"
                          label="Group key"
                          control={methods.control}
                          placeholder="dimension"
                          className="max-w-xs"
                        />
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
