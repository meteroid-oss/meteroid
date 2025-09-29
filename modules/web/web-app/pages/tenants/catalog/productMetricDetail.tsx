import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
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
  ChevronLeftIcon,
  CopyIcon,
  EditIcon,
  EyeIcon,
} from 'lucide-react'
import { ReactNode, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

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

// Status Badge Component
const StatusBadge = ({ isArchived }: { isArchived: boolean }) => {
  return (
    <Badge variant={isArchived ? 'secondary' : 'success'}>
      {isArchived ? 'Archived' : 'Active'}
    </Badge>
  )
}

// Section Title Component
const SectionTitle = ({ children }: { children: ReactNode }) => (
  <h3 className="text-lg font-medium text-foreground mb-3">{children}</h3>
)

// Detail Row Component
const DetailRow = ({ label, value, link }: { label: string; value: ReactNode; link?: string }) => (
  <div className="text-[13px] flex justify-between py-2 border-b border-border last:border-0">
    <div className="text-muted-foreground">{label}</div>
    {link ? (
      <Link to={link}>
        <div className="font-medium text-brand hover:underline">{value}</div>
      </Link>
    ) : (
      <div className="font-medium text-foreground">{value}</div>
    )}
  </div>
)

// Detail Section Component
const DetailSection = ({ title, children }: { title: string; children: ReactNode }) => (
  <div className="mb-6">
    <SectionTitle>{title}</SectionTitle>
    <div className="space-y-1">{children}</div>
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
      await queryClient.invalidateQueries({ queryKey: [listBillableMetrics.service.typeName] })
      await queryClient.invalidateQueries({ queryKey: [getBillableMetric.service.typeName] })
      toast.success('Metric archived successfully')
    },
    onError: () => {
      toast.error('Failed to archive metric')
    },
  })

  const unarchiveMutation = useMutation(unarchiveBillableMetric, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listBillableMetrics.service.typeName] })
      await queryClient.invalidateQueries({ queryKey: [getBillableMetric.service.typeName] })
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
      <div className="p-6">
        <Skeleton height={16} width={50} className="mb-4" />
        <div className="flex gap-6">
          <div className="flex-1">
            <Skeleton height={100} className="mb-4" />
            <Skeleton height={200} className="mb-4" />
          </div>
          <div className="w-80">
            <Skeleton height={300} className="mb-4" />
          </div>
        </div>
      </div>
    )
  }

  const isArchived = !!data.archivedAt
  const aggregationType = data.aggregation?.aggregationType || Aggregation_AggregationType.SUM
  const aggregationKey = data.aggregation?.aggregationKey

  return (
    <div className="flex min-h-screen bg-background gap-2">
      {/* Main content area */}
      <div className="flex-1 p-6 pr-0">
        <div className="flex items-center mb-6 w-full justify-between">
          <div className="flex items-center">
            <ChevronLeftIcon
              className="cursor-pointer text-muted-foreground hover:text-foreground mr-2"
              onClick={() => navigate(`${basePath}/metrics`)}
              size={20}
            />
            <h2 className="text-xl font-semibold text-foreground">{data.name}</h2>
            <div className="ml-2">
              <StatusBadge isArchived={isArchived} />
            </div>
          </div>
          <div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="primary" className="gap-2" size="sm" hasIcon>
                  Actions <ChevronDown className="w-4 h-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem disabled>
                  <EditIcon size={16} className="mr-2" />
                  Edit Metric
                </DropdownMenuItem>
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
        </div>

        {/* Overview card */}
        <div className="bg-card rounded-lg shadow-sm p-6 mb-6">
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-medium text-foreground">Overview</h3>
          </div>
          <div className="grid grid-cols-1 gap-6">
            <div className="grid grid-cols-3 gap-6">
              <div className="border-r border-border pr-4 last:border-0">
                <div className="text-sm text-muted-foreground">Event Name</div>
                <div className="text-md font-medium mt-1">
                  <code className="bg-muted px-2 py-1 rounded text-sm">{data.code}</code>
                </div>
              </div>
              <div className="border-r border-border pr-4 last:border-0">
                <div className="text-sm text-muted-foreground">Aggregation</div>
                <div className="text-md font-medium mt-1">
                  <code className="bg-muted px-2 py-1 rounded text-sm">
                    {aggregationTypeMapper[aggregationType]}
                    {aggregationKey && <>({aggregationKey})</>}
                  </code>
                </div>
              </div>
              <div>
                <div className="text-sm text-muted-foreground">Status</div>
                <div className="text-md font-medium mt-1">
                  <StatusBadge isArchived={isArchived} />
                </div>
              </div>
            </div>
            {data.description && (
              <div className="border-t border-border pt-4">
                <div className="text-sm text-muted-foreground">Description</div>
                <div className="text-md font-medium mt-1 whitespace-pre-line text-foreground">
                  {data.description}
                </div>
              </div>
            )}
          </div>
        </div>

        {/* Analytics placeholder - Future implementation */}
        <div className="bg-card rounded-lg shadow-sm p-6 mb-6">
          <div className="flex justify-between items-center mb-4">
            <h3 className="text-lg font-medium text-foreground">Usage Analytics</h3>
          </div>
          <div className="flex items-center justify-center h-32 text-muted-foreground">
            <div className="text-center">
              <div className="text-sm">Analytics coming soon</div>
              <div className="text-xs mt-1">View usage data and trends for this metric</div>
            </div>
          </div>
        </div>

        {/* Customer Data placeholder - Future implementation */}
        <div className="bg-card rounded-lg shadow-sm mb-6">
          <div className="p-4 border-b border-border">
            <h3 className="text-md font-medium text-foreground">Customer Data</h3>
          </div>
          <div className="p-6">
            <div className="flex items-center justify-center h-32 text-muted-foreground">
              <div className="text-center">
                <div className="text-sm">Customer data coming soon</div>
                <div className="text-xs mt-1">View how customers are using this metric</div>
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* Sidebar */}
      <div className="w-80 p-6 border-l border-border">
        <DetailSection title="Metric Details">
          <DetailRow label="ID" value={data.id} />
          <DetailRow label="Name" value={data.name} />
          <DetailRow label="Event Name" value={data.code} />
          <DetailRow label="Status" value={<StatusBadge isArchived={isArchived} />} />
        </DetailSection>

        <DetailSection title="Aggregation">
          <DetailRow label="Type" value={aggregationTypeMapper[aggregationType]} />
          {aggregationKey && <DetailRow label="Key" value={aggregationKey} />}
          {data.aggregation?.unitConversion && (
            <DetailRow
              label="Unit Conversion"
              value={`${data.aggregation.unitConversion.factor}x (${unitConversionRoundingMapper[data.aggregation.unitConversion.rounding] || 'none'})`}
            />
          )}
        </DetailSection>

        {data.usageGroupKey && (
          <DetailSection title="Usage Grouping">
            <DetailRow label="Group Key" value={data.usageGroupKey} />
          </DetailSection>
        )}

        {data.segmentationMatrix && data.segmentationMatrix.matrix && (
          <DetailSection title="Segmentation">
            <DetailRow
              label="Type"
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
                    label="Dimension"
                    value={data.segmentationMatrix.matrix.value.dimension?.key || 'N/A'}
                  />
                  <DetailRow
                    label="Values"
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
                    label="Dimension 1"
                    value={data.segmentationMatrix.matrix.value.dimension1?.key || 'N/A'}
                  />
                  <DetailRow
                    label="Dimension 1 Values"
                    value={
                      data.segmentationMatrix.matrix.value.dimension1?.values &&
                      data.segmentationMatrix.matrix.value.dimension1.values.length > 5
                        ? `${data.segmentationMatrix.matrix.value.dimension1.values.slice(0, 5).join(', ')}... (${data.segmentationMatrix.matrix.value.dimension1.values.length} total)`
                        : data.segmentationMatrix.matrix.value.dimension1?.values?.join(', ') ||
                          'N/A'
                    }
                  />
                  <DetailRow
                    label="Dimension 2"
                    value={data.segmentationMatrix.matrix.value.dimension2?.key || 'N/A'}
                  />
                  <DetailRow
                    label="Dimension 2 Values"
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
                    label="Primary Dimension"
                    value={data.segmentationMatrix.matrix.value.dimensionKey || 'N/A'}
                  />
                  <DetailRow
                    label="Linked Dimension"
                    value={data.segmentationMatrix.matrix.value.linkedDimensionKey || 'N/A'}
                  />

                  <DetailRow
                    label="Values"
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
          </DetailSection>
        )}

        <DetailSection title="Timeline">
          <DetailRow label="Created At" value={data.createdAt?.toDate().toLocaleString()} />
          {data.archivedAt && (
            <DetailRow label="Archived At" value={data.archivedAt?.toDate().toLocaleString()} />
          )}
        </DetailSection>

        {/* Plans using this metric - Future implementation */}
        {/* <DetailSection title="Used in Plans">
          <div className="text-xs text-muted-foreground">
            Plans using this metric will be shown here
          </div>
        </DetailSection> */}
      </div>

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
    </div>
  )
}
