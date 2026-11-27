import { timestampDate } from '@bufbuild/protobuf/wkt'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Card,
  CardContent,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Skeleton,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import {
  ArchiveIcon,
  ArchiveRestoreIcon,
  ChevronDown,
  CopyIcon,
  EditIcon,
  EyeIcon,
  Gauge,
} from 'lucide-react'
import { ReactNode, useState } from 'react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'

import { Gate } from '@/components/Gate'
import { LocalId } from '@/components/LocalId'
import {
  DetailColumns,
  DetailIcon,
  DetailMain,
  DetailPageFrame,
  DetailRow,
  DetailSection,
  DetailSections,
  DetailSidebar,
  DetailTitle,
  SidebarSection,
  SidebarSeparator,
} from '@/components/layouts/DetailLayout'
import { EntityActivityTimeline } from '@/features/activity/EntityActivityTimeline'
import { BillableMetricStatusBadge } from '@/features/productCatalog/metrics/BillableMetricStatusBadge'
import { MetricCustomersCard } from '@/features/productCatalog/metrics/MetricCustomersCard'
import { MetricPreview } from '@/features/productCatalog/metrics/MetricPreview'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import {
  archiveBillableMetric,
  getBillableMetric,
  listBillableMetrics,
  unarchiveBillableMetric,
} from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  Aggregation_AggregationType,
  Aggregation_UnitConversion_UnitConversionRounding,
} from '@/rpc/api/billablemetrics/v1/models_pb'
import { useTypedParams } from '@/utils/params'

const aggregationTypeMapper: Record<Aggregation_AggregationType, string> = {
  [Aggregation_AggregationType.SUM]: 'sum',
  [Aggregation_AggregationType.MIN]: 'min',
  [Aggregation_AggregationType.MAX]: 'max',
  [Aggregation_AggregationType.MEAN]: 'mean',
  [Aggregation_AggregationType.LATEST]: 'latest',
  [Aggregation_AggregationType.COUNT]: 'count',
  [Aggregation_AggregationType.COUNT_DISTINCT]: 'distinct',
}

const unitConversionRoundingMapper: Record<
  Aggregation_UnitConversion_UnitConversionRounding,
  string
> = {
  [Aggregation_UnitConversion_UnitConversionRounding.NONE]: 'none',
  [Aggregation_UnitConversion_UnitConversionRounding.UP]: 'up',
  [Aggregation_UnitConversion_UnitConversionRounding.DOWN]: 'down',
  [Aggregation_UnitConversion_UnitConversionRounding.NEAREST]: 'nearest',
}

/** One fact of the overview: a compact tile, raised as the customer page's cards. */
const OverviewFact = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="flex min-w-0 flex-col gap-2 rounded-lg border border-border bg-card p-4 dark:bg-foreground/[0.035]">
    <dt className="text-[13px] leading-none tracking-[-0.13px] text-foreground/60">{label}</dt>
    <dd className="truncate text-foreground">{children}</dd>
  </div>
)

export const ProductMetricDetail = () => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  const queryClient = useQueryClient()
  const { metricId } = useTypedParams<{ metricId: string }>()
  const [showSegmentationModal, setShowSegmentationModal] = useState(false)

  const metricQuery = useQuery(
    getBillableMetric,
    {
      id: metricId ?? '',
    },
    { enabled: Boolean(metricId) }
  )

  const archiveMutation = useMutation(archiveBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listBillableMetrics.parent,
          cardinality: undefined,
        }),
      })
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getBillableMetric.parent,
          cardinality: undefined,
        }),
      })
      toast.success('Metric archived successfully')
    },
    onError: () => {
      toast.error('Failed to archive metric')
    },
  })

  const unarchiveMutation = useMutation(unarchiveBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listBillableMetrics.parent,
          cardinality: undefined,
        }),
      })
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: getBillableMetric.parent,
          cardinality: undefined,
        }),
      })
      toast.success('Metric unarchived successfully')
    },
    onError: () => {
      toast.error('Failed to unarchive metric')
    },
  })

  const handleArchive = () => {
    if (data) {
      archiveMutation.mutate({ id: data.id })
    }
  }

  const handleUnarchive = () => {
    if (data) {
      unarchiveMutation.mutate({ id: data.id })
    }
  }

  const data = metricQuery.data?.billableMetric
  const isLoading = metricQuery.isLoading

  if (isLoading || !data) {
    return (
      <DetailPageFrame section={{ label: 'Metrics', to: `${basePath}/metrics` }} title="…">
        <div className="flex flex-col gap-2 p-6">
          <Skeleton height={16} width={50} />
          <Skeleton height={44} />
        </div>
      </DetailPageFrame>
    )
  }

  const isArchived = !!data.archivedAt
  const aggregationType = data.aggregation?.aggregationType || Aggregation_AggregationType.SUM
  const aggregationKey = data.aggregation?.aggregationKey

  const unitConversion = data.aggregation?.unitConversion

  return (
    <DetailPageFrame
      section={{ label: 'Metrics', to: `${basePath}/metrics` }}
      title={
        <span className="flex items-center gap-2">
          <span className="truncate">{data.name}</span>
          <BillableMetricStatusBadge isArchived={isArchived} />
        </span>
      }
      actions={
        <Gate perm="catalog:manage">
          <div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="secondary" className="gap-1.5 pr-2" size="sm">
                  Actions <ChevronDown className="h-3.5 w-3.5 text-muted-foreground" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {!isArchived && (
                  <DropdownMenuItem onClick={() => navigate(`${basePath}/metrics/edit/${data.id}`)}>
                    <EditIcon size={16} className="mr-2" />
                    Edit Metric
                  </DropdownMenuItem>
                )}
                {isArchived ? (
                  <DropdownMenuItem onClick={handleUnarchive}>
                    <ArchiveRestoreIcon size={16} className="mr-2" />
                    Unarchive
                  </DropdownMenuItem>
                ) : (
                  <DropdownMenuItem onClick={handleArchive}>
                    <ArchiveIcon size={16} className="mr-2" />
                    Archive
                  </DropdownMenuItem>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </Gate>
      }
    >
      <DetailColumns>
        <DetailMain>
          <DetailTitle
            media={
              <DetailIcon>
                <Gauge />
              </DetailIcon>
            }
            title={data.name}
            subtitle={<span className="font-mono">{data.code}</span>}
          />
          {data.description && (
            <p className="-mt-2 mb-2 max-w-3xl whitespace-pre-line text-[13px] leading-relaxed text-muted-foreground">
              {data.description}
            </p>
          )}
          {/* The overview: what the metric counts and how, and whether it is in use. */}
          <dl aria-label="Metric overview" className="mb-4 grid grid-cols-1 gap-3 sm:grid-cols-3">
            <OverviewFact label="Event name">
              <code className="font-mono text-[13px]">{data.code}</code>
            </OverviewFact>
            <OverviewFact label="Aggregation">
              <code className="font-mono text-[13px]">
                {aggregationTypeMapper[aggregationType]}
                {aggregationKey && <>({aggregationKey})</>}
              </code>
            </OverviewFact>
            <OverviewFact label="Status">
              <BillableMetricStatusBadge isArchived={isArchived} />
            </OverviewFact>
          </dl>
          <DetailSections>
            {/* Usage analytics: chart, period aggregate, and the raw events feeding it. */}
            <DetailSection title="Usage">
              <MetricPreview
                code={data.code}
                aggregation={data.aggregation}
                segmentationMatrix={data.segmentationMatrix}
                filters={data.filters}
                usageGroupKey={data.usageGroupKey}
                exploreLinkKey={`bm:${data.id}`}
              />
            </DetailSection>
            {/* How customers are using this metric. */}
            <DetailSection title="Customers">
              <MetricCustomersCard metricId={data.id} aggregationType={aggregationType} />
            </DetailSection>
          </DetailSections>
        </DetailMain>

        <DetailSidebar>
          <SidebarSection
            title="Details"
            action={
              !isArchived && (
                <Gate perm="catalog:manage">
                  <Button
                    size="sm"
                    variant="secondary"
                    onClick={() => navigate(`${basePath}/metrics/edit/${data.id}`)}
                  >
                    Edit
                  </Button>
                </Gate>
              )
            }
          >
            <DetailRow title="ID" value={<LocalId localId={data.id} />} />
            <DetailRow title="Name" value={data.name} />
            <DetailRow title="Event name" value={<span className="font-mono">{data.code}</span>} />
            <DetailRow
              title="Status"
              value={<BillableMetricStatusBadge isArchived={isArchived} />}
            />
            <DetailRow
              title="Created"
              value={data.createdAt && timestampDate(data.createdAt).toLocaleString()}
            />
            {data.archivedAt && (
              <DetailRow title="Archived" value={timestampDate(data.archivedAt).toLocaleString()} />
            )}
          </SidebarSection>

          <SidebarSeparator />
          <SidebarSection title="Aggregation">
            <DetailRow title="Type" value={aggregationTypeMapper[aggregationType]} />
            {aggregationKey && (
              <DetailRow title="Key" value={<span className="font-mono">{aggregationKey}</span>} />
            )}
            {unitConversion && (
              <DetailRow
                title="Unit conversion"
                value={`${unitConversion.factor}x (${unitConversionRoundingMapper[unitConversion.rounding] || 'none'})`}
              />
            )}
          </SidebarSection>

          {(data.usageGroupKey || data.usageBreakdownKey) && (
            <>
              <SidebarSeparator />
              <SidebarSection title="Grouping">
                {data.usageGroupKey && (
                  <DetailRow
                    title="Group key"
                    value={<span className="font-mono">{data.usageGroupKey}</span>}
                  />
                )}
                {data.usageBreakdownKey && (
                  <DetailRow
                    title="Breakdown key"
                    value={<span className="font-mono">{data.usageBreakdownKey}</span>}
                  />
                )}
              </SidebarSection>
            </>
          )}

          {data.segmentationMatrix && data.segmentationMatrix.matrix && (
            <>
              <SidebarSeparator />
              <SidebarSection title="Segmentation">
                <DetailRow
                  title="Type"
                  value={
                    data.segmentationMatrix.matrix.case === 'single'
                      ? 'Single dimension'
                      : data.segmentationMatrix.matrix.case === 'double'
                        ? 'Two dimensions (independent)'
                        : data.segmentationMatrix.matrix.case === 'linked'
                          ? 'Two dimensions (dependent)'
                          : 'Unknown'
                  }
                />
                {data.segmentationMatrix.matrix.case === 'single' &&
                  data.segmentationMatrix.matrix.value && (
                    <>
                      <DetailRow
                        title="Dimension"
                        value={data.segmentationMatrix.matrix.value.dimension?.key || 'N/A'}
                      />
                      <DetailRow
                        title="Values"
                        value={
                          data.segmentationMatrix.matrix.value.dimension?.values &&
                          data.segmentationMatrix.matrix.value.dimension.values.length > 5
                            ? `${data.segmentationMatrix.matrix.value.dimension.values.slice(0, 5).join(', ')}... (${data.segmentationMatrix.matrix.value.dimension.values.length} total)`
                            : data.segmentationMatrix.matrix.value.dimension?.values?.join(', ') ||
                              'N/A'
                        }
                      />
                    </>
                  )}
                {data.segmentationMatrix.matrix.case === 'double' &&
                  data.segmentationMatrix.matrix.value && (
                    <>
                      <DetailRow
                        title="Dimension 1"
                        value={data.segmentationMatrix.matrix.value.dimension1?.key || 'N/A'}
                      />
                      <DetailRow
                        title="Dimension 1 Values"
                        value={
                          data.segmentationMatrix.matrix.value.dimension1?.values &&
                          data.segmentationMatrix.matrix.value.dimension1.values.length > 5
                            ? `${data.segmentationMatrix.matrix.value.dimension1.values.slice(0, 5).join(', ')}... (${data.segmentationMatrix.matrix.value.dimension1.values.length} total)`
                            : data.segmentationMatrix.matrix.value.dimension1?.values?.join(', ') ||
                              'N/A'
                        }
                      />
                      <DetailRow
                        title="Dimension 2"
                        value={data.segmentationMatrix.matrix.value.dimension2?.key || 'N/A'}
                      />
                      <DetailRow
                        title="Dimension 2 Values"
                        value={
                          data.segmentationMatrix.matrix.value.dimension2?.values &&
                          data.segmentationMatrix.matrix.value.dimension2.values.length > 5
                            ? `${data.segmentationMatrix.matrix.value.dimension2.values.slice(0, 5).join(', ')}... (${data.segmentationMatrix.matrix.value.dimension2.values.length} total)`
                            : data.segmentationMatrix.matrix.value.dimension2?.values?.join(', ') ||
                              'N/A'
                        }
                      />
                    </>
                  )}
                {data.segmentationMatrix.matrix.case === 'linked' &&
                  data.segmentationMatrix.matrix.value && (
                    <>
                      <DetailRow
                        title="Primary Dimension"
                        value={data.segmentationMatrix.matrix.value.dimensionKey || 'N/A'}
                      />
                      <DetailRow
                        title="Linked Dimension"
                        value={data.segmentationMatrix.matrix.value.linkedDimensionKey || 'N/A'}
                      />

                      <DetailRow
                        title="Values"
                        value={
                          <Button
                            variant="ghost"
                            size="icon"
                            onClick={() => setShowSegmentationModal(true)}
                            className="gap-2"
                          >
                            <EyeIcon className="h-4 w-4" />
                          </Button>
                        }
                      />
                    </>
                  )}
              </SidebarSection>
            </>
          )}

          <SidebarSeparator />
          <SidebarSection title="Activity">
            <EntityActivityTimeline
              entityType="billable_metric"
              entityId={data.id}
              emptyLabel="No activity yet for this metric"
              timeline
            />
          </SidebarSection>
        </DetailSidebar>
      </DetailColumns>

      {/* Segmentation Values Modal */}
      <Dialog open={showSegmentationModal} onOpenChange={setShowSegmentationModal}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Segmentation Values</DialogTitle>
            <DialogDescription>
              {data?.segmentationMatrix?.matrix?.case === 'linked' &&
                'Linked dimension values mapping'}
            </DialogDescription>
          </DialogHeader>
          {data?.segmentationMatrix?.matrix?.case === 'linked' &&
            data.segmentationMatrix.matrix.value && (
              <div className="space-y-4">
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <div className="text-sm font-medium">Primary Dimension</div>
                    <div className="text-sm text-muted-foreground">
                      {data.segmentationMatrix.matrix.value.dimensionKey}
                    </div>
                  </div>
                  <div>
                    <div className="text-sm font-medium">Linked Dimension</div>
                    <div className="text-sm text-muted-foreground">
                      {data.segmentationMatrix.matrix.value.linkedDimensionKey}
                    </div>
                  </div>
                </div>
                <div>
                  <div className="text-sm font-medium mb-2">Values Mapping</div>
                  <Card>
                    <CardContent className="p-4 relative">
                      <Button
                        variant="ghost"
                        size="icon"
                        onClick={() => {
                          if (
                            data.segmentationMatrix?.matrix?.case === 'linked' &&
                            data.segmentationMatrix.matrix.value &&
                            'values' in data.segmentationMatrix.matrix.value
                          ) {
                            const jsonData = JSON.stringify(
                              Object.entries(
                                data.segmentationMatrix.matrix.value.values || {}
                              ).reduce(
                                (acc, [key, value]) => {
                                  acc[key] = value.values || []
                                  return acc
                                },
                                {} as Record<string, string[]>
                              ),
                              null,
                              2
                            )
                            copyToClipboard(jsonData)
                            toast.success('Copied to clipboard')
                          }
                        }}
                        className="absolute top-2 right-2 h-8 w-8"
                      >
                        <CopyIcon className="h-4 w-4" />
                      </Button>
                      <pre className="text-xs overflow-auto max-h-96">
                        {data.segmentationMatrix?.matrix?.case === 'linked' &&
                          data.segmentationMatrix.matrix.value &&
                          'values' in data.segmentationMatrix.matrix.value &&
                          JSON.stringify(
                            Object.entries(
                              data.segmentationMatrix.matrix.value.values || {}
                            ).reduce(
                              (acc, [key, value]) => {
                                acc[key] = value.values || []
                                return acc
                              },
                              {} as Record<string, string[]>
                            ),
                            null,
                            2
                          )}
                      </pre>
                    </CardContent>
                  </Card>
                </div>
              </div>
            )}
        </DialogContent>
      </Dialog>
    </DetailPageFrame>
  )
}
