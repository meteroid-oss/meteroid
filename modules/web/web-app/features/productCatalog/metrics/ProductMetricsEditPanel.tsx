import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import { Flex, FormItem, Input, Modal, SidePanel, Textarea } from '@md/ui'
import { pipe, D } from '@mobily/ts-belt'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'

import { AccordionPanel } from '@/components/AccordionPanel'
import ConfirmationModal from '@/components/ConfirmationModal'
import { AggregationSection } from '@/features/productCatalog/metrics/AggregationSection'
import { SegmentationMatrixSection } from '@/features/productCatalog/metrics/SegmentationMatrixSection'
import { UnitConversionSection } from '@/features/productCatalog/metrics/UnitConversionSection'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import {
  createBillableMetric,
  listBillableMetrics,
} from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  Aggregation_AggregationType,
  Aggregation_UnitConversion_UnitConversionRounding,
} from '@/rpc/api/billablemetrics/v1/models_pb'
import { useTypedParams } from '@/utils/params'

interface ProductMetricsEditPanelProps {
  visible: boolean
  closePanel: () => void
}

// TODO https://doc.getlago.com/docs/guide/billable-metrics/dimensions
// Add Dimension matrix ()
// One is fixed => allow custom pricing
// The other is dynamic (group key) => only for invoice
export const ProductMetricsEditPanel = ({ visible, closePanel }: ProductMetricsEditPanelProps) => {
  const [isClosingPanel, setIsClosingPanel] = useState(false)

  const queryClient = useQueryClient()

  const createBillableMetricMut = useMutation(createBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listBillableMetrics) })
    },
  })
  const { familyExternalId } = useTypedParams<{ familyExternalId: string }>()

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
    mode: 'onChange',
  })

  useEffect(() => {
    console.log('errors', methods.formState.errors)
  }, [methods.formState.errors])

  const safeClosePanel = () => {
    const isDirty = methods.formState.isDirty
    if (isDirty) {
      setIsClosingPanel(true)
    } else {
      methods.reset()
      closePanel()
    }
  }

  // TODO try without the form, with onConfirm
  return (
    <>
      <SidePanel
        size="xlarge"
        key="TableEditor"
        visible={visible}
        header={<SidePanel.HeaderTitle>Register a new metric</SidePanel.HeaderTitle>}
        className={`transition-all duration-100 ease-in `}
        onCancel={safeClosePanel}
        onConfirm={methods.handleSubmit(async input => {
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
            familyExternalId,
          })
          methods.reset()
          closePanel()
        })}
        onInteractOutside={event => {
          const isToast = (event.target as Element)?.closest('#toast')
          if (isToast) {
            event.preventDefault()
          }
        }}
      >
        <SidePanel.Content>
          <Flex direction="column" gap={spaces.space7}>
            <FormItem
              name="name"
              label="Metric name"
              error={methods.formState.errors.metricName?.message}
            >
              <Input
                type="text"
                placeholder="Compute (GB-hr)"
                {...methods.register('metricName')}
                className="max-w-xs"
              />
            </FormItem>

            <FormItem
              name="code"
              label="Event Code"
              error={methods.formState.errors.eventCode?.message}
              hint={
                <>
                  Qualifies an event stream, ex: page_views.
                  <br />A single usage event can be used for multiple metrics.
                </>
              }
            >
              <Input
                type="text"
                placeholder="compute_usage"
                {...methods.register('eventCode')}
                className="max-w-xs"
              />
            </FormItem>

            <FormItem
              name="description"
              label="Description"
              error={methods.formState.errors.metricDescription?.message}
            >
              <Textarea placeholder="description" {...methods.register('metricDescription')} />
            </FormItem>
          </Flex>
        </SidePanel.Content>
        <AggregationSection methods={methods} />
        <UnitConversionSection methods={methods} />
        <SegmentationMatrixSection methods={methods} />
        <SidePanel.Separator />
        <SidePanel.Content>
          <AccordionPanel
            title={
              <div className="space-x-4 items-center flex pr-4">
                <h3>Usage Group Key</h3>
                <span className="text-xs text-muted-foreground">optional</span>
              </div>
            }
            defaultOpen={false}
          >
            <div className="space-y-6">
              <div className="text-sm text-slate-1000 space-y-2">
                <p>
                  Specify a dimension to group items by in the invoice and usage endpoints.
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
              </div>
              <div>
                <FormItem label="Group key" {...methods.withError('usageGroupKey')}>
                  <Input
                    className="max-w-sm"
                    placeholder="dimension"
                    {...methods.register('usageGroupKey')}
                  />
                </FormItem>
              </div>
            </div>
          </AccordionPanel>
        </SidePanel.Content>
      </SidePanel>
      <ConfirmationModal
        visible={isClosingPanel}
        header="Confirm to close"
        buttonLabel="Confirm"
        onSelectCancel={() => setIsClosingPanel(false)}
        onSelectConfirm={() => {
          setIsClosingPanel(false)
          methods.reset()
          closePanel()
        }}
      >
        <Modal.Content>
          <p className="py-4 text-sm text-muted-foreground">
            There are unsaved changes. Are you sure you want to close the panel? Your changes will
            be lost.
          </p>
        </Modal.Content>
      </ConfirmationModal>
    </>
  )
}
