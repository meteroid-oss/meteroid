import { Skeleton, TableCell, TableRow } from '@md/ui'
import { useEffect } from 'react'

import { useResourceTypes } from '@/features/resources/hooks'
import { useQuery } from '@/lib/connectrpc'
import { formatUsage } from '@/lib/utils/numbers'
import { previewMetricAttribution } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'

import type { PreviewMetricAttributionResponse } from '@/rpc/api/billablemetrics/v1/billablemetrics_pb'

/** Unset for the customer; a type, read from `property` or else from the type's keys. */
export type Attribution = { typeId: string; property?: string } | undefined

export const attributionInit = (a: Attribution) =>
  a ? { resourceTypeId: a.typeId, property: a.property || undefined } : undefined

const usePreview = (metricId: string, value: Attribution, onReady?: (ready: boolean) => void) => {
  const preview = useQuery(previewMetricAttribution, { id: metricId, attributedTo: attributionInit(value) })
  useEffect(() => {
    onReady?.(!!preview.data)
  }, [preview.data]) // eslint-disable-line react-hooks/exhaustive-deps
  return preview
}

/** Shares of the usage for a sum or count; otherwise what each side bills, which don't add up. */
const sharesOf = (p: PreviewMetricAttributionResponse) => {
  const resource = Number(p.resourceSubscriptions)
  const customer = Number(p.customerSubscriptions)
  const none = Number(p.notBilled)
  const total = resource + customer + none
  const pct = (q: number) => (total ? `${Math.round((q / total) * 100)}%` : '0%')
  const show = p.additive ? pct : formatUsage
  return { resource: show(resource), customer: show(customer), none: show(none) }
}

/** The last 7 days split by who would bill them under `value`. */
export const AttributionPreview = ({
  metricId,
  value,
  details,
  onReady,
}: {
  metricId: string
  value: Attribution
  /** The sample size and the drafts a save recomputes. */
  details?: boolean
  onReady?: (ready: boolean) => void
}) => {
  const { byId } = useResourceTypes()
  const type = value ? byId.get(value.typeId) : undefined
  const preview = usePreview(metricId, value, onReady)

  if (preview.isLoading) return <Skeleton className="h-16 w-full" />
  const p = preview.data
  if (!p) return <p className="text-sm text-destructive">{preview.error?.rawMessage}</p>
  const shares = sharesOf(p)
  const meta = [
    'Last 7 days',
    details && p.sampled ? `${p.sampledCustomers} customers sampled` : undefined,
    details ? `${p.draftInvoices} draft invoice${p.draftInvoices === 1 ? '' : 's'} recomputed on save` : undefined,
  ].filter(Boolean)

  return (
    <div className="flex flex-col gap-1.5">
      <dl className="grid grid-cols-3 gap-x-4 gap-y-2 rounded-md border border-border p-3 text-[13px]">
        <div>
          <dt className="text-xs text-muted-foreground">{type?.name ?? 'Resource'} subscriptions</dt>
          <dd className="font-medium tabular-nums">{shares.resource}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Customer subscriptions</dt>
          <dd className="font-medium tabular-nums">{shares.customer}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Unattributed</dt>
          <dd className="font-medium tabular-nums">{shares.none}</dd>
        </div>
      </dl>
      <p className="text-xs text-muted-foreground">{meta.join(' · ')}</p>
    </div>
  )
}

/** One metric's split as a table row: name, resource share, customer share, unattributed share. */
export const AttributionPreviewRow = ({
  metricId,
  metricName,
  value,
}: {
  metricId: string
  metricName: string
  value: Attribution
}) => {
  const preview = usePreview(metricId, value)
  const p = preview.data
  const shares = p && sharesOf(p)
  const cell = (v: string | undefined) => (
    <TableCell className="text-right text-[13px] tabular-nums">
      {preview.isLoading ? <Skeleton className="ml-auto h-4 w-10" /> : (v ?? '—')}
    </TableCell>
  )
  return (
    <TableRow>
      <TableCell className="text-sm font-medium">
        {metricName}
        {preview.error && <div className="text-xs font-normal text-destructive">{preview.error.rawMessage}</div>}
      </TableCell>
      {cell(shares?.resource)}
      {cell(shares?.customer)}
      {cell(shares?.none)}
    </TableRow>
  )
}
