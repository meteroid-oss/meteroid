import { Code } from '@connectrpc/connect'
import { skipToken } from '@connectrpc/connect-query'
import { Badge, Button, cn, Tabs, TabsList, TabsTrigger } from '@md/ui'
import { ColumnDef } from '@tanstack/react-table'
import { formatDistanceToNow } from 'date-fns'
import { Eye } from 'lucide-react'
import { useMemo, useState } from 'react'

import { StandardTable } from '@/components/table/StandardTable'
import { EntityHoverCard } from '@/features/activity/EntityHoverCard'
import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { useQueryState } from '@/hooks/useQueryState'
import { useQuery } from '@/lib/connectrpc'
import {
  listSentEmails,
  listSkippedEmails,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import {
  EmailKind,
  SentEmailSummary,
  SkippedEmailSummary,
} from '@/rpc/api/customeremails/v1/models_pb'
import { parseAndFormatDateTime } from '@/utils/date'

import { EMAIL_KIND_META, emailKindLabel } from './EmailKindsView'
import { EmailPreviewDialog } from './EmailPreviewDialog'
import { localeLabel } from './EmailPreviewPanel'
import { skipReason } from './skipReasons'

const PAGE_SIZE = 20

const FilterChip = ({
  active,
  onClick,
  children,
}: {
  active: boolean
  onClick: () => void
  children: string
}) => (
  <button
    type="button"
    onClick={onClick}
    className={cn(
      'rounded-full border px-2.5 py-0.5 text-xs transition-colors',
      active
        ? 'border-foreground bg-foreground text-background'
        : 'border-border text-muted-foreground hover:border-foreground/40 hover:text-foreground'
    )}
  >
    {children}
  </button>
)

const retryUnlessUnimplemented = (count: number, err: { code: Code }) =>
  err.code !== Code.Unimplemented && count < 2

const When = ({ at }: { at: string }) => (
  <span className="whitespace-nowrap text-muted-foreground" title={parseAndFormatDateTime(at)}>
    {formatDistanceToNow(new Date(at), { addSuffix: true })}
  </span>
)

const EntityCell = ({ email }: { email: SentEmailSummary | SkippedEmailSummary }) => (
  <EntityHoverCard
    entityType={email.entityType}
    entityId={email.entityId}
    label={email.entityLabel ?? email.entityType.replace(/_/g, ' ')}
  />
)

const Pager = ({
  cursors,
  setCursors,
  nextCursor,
}: {
  cursors: string[]
  setCursors: (update: (c: string[]) => string[]) => void
  nextCursor?: string
}) =>
  cursors.length > 0 || nextCursor ? (
    <div className="flex items-center justify-between text-xs text-muted-foreground">
      <span>Page {cursors.length + 1}</span>
      <div className="flex gap-2">
        <Button
          variant="outline"
          size="sm"
          disabled={cursors.length === 0}
          onClick={() => setCursors(c => c.slice(0, -1))}
        >
          Previous
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={!nextCursor}
          onClick={() => nextCursor && setCursors(c => [...c, nextCursor])}
        >
          Next
        </Button>
      </div>
    </div>
  ) : null

type ListProps = { entityId?: string; kind?: EmailKind }

export const SentEmailsView = () => {
  const [log, setLog] = useQueryState('log', 'sent', { replace: true })
  const [kind, setKind] = useState<EmailKind | undefined>()
  const { selectedEntityId: entityId } = useInvoicingEntity()

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-3">
        <Tabs value={log} onValueChange={setLog}>
          <TabsList className="h-8">
            <TabsTrigger value="sent" className="text-xs">
              Sent
            </TabsTrigger>
            <TabsTrigger value="skipped" className="text-xs">
              Skipped
            </TabsTrigger>
          </TabsList>
        </Tabs>
        <div className="flex flex-wrap items-center gap-1.5">
          <FilterChip active={kind === undefined} onClick={() => setKind(undefined)}>
            All
          </FilterChip>
          {EMAIL_KIND_META.map(meta => (
            <FilterChip
              key={meta.kind}
              active={kind === meta.kind}
              onClick={() => setKind(meta.kind)}
            >
              {meta.label}
            </FilterChip>
          ))}
        </div>
      </div>

      {/* Keyed so a new filter starts back on the first page. */}
      {log === 'skipped' ? (
        <SkippedList key={`${entityId}:${kind}`} entityId={entityId} kind={kind} />
      ) : (
        <SentList key={`${entityId}:${kind}`} entityId={entityId} kind={kind} />
      )}
    </div>
  )
}

const SentList = ({ entityId, kind }: ListProps) => {
  const [cursors, setCursors] = useState<string[]>([])
  const [previewId, setPreviewId] = useState<string | undefined>()

  const currentCursor = cursors[cursors.length - 1]
  const query = useQuery(
    listSentEmails,
    entityId
      ? { invoicingEntityId: entityId, kind, cursor: currentCursor, limit: PAGE_SIZE }
      : skipToken,
    { retry: retryUnlessUnimplemented }
  )
  const emails = query.data?.emails ?? []
  const nextCursor = query.data?.nextCursor
  const unavailable = query.error?.code === Code.Unimplemented

  const columns = useMemo<ColumnDef<SentEmailSummary>[]>(
    () => [
      {
        header: 'Sent',
        accessorKey: 'sentAt',
        cell: ({ row }) => <When at={row.original.sentAt} />,
      },
      {
        header: 'Email',
        accessorKey: 'subject',
        cell: ({ row }) => (
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="truncate font-medium">{row.original.subject}</span>
            <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
              {emailKindLabel(row.original.kind)}
              {row.original.resentFrom && (
                <Badge variant="outline" size="sm" className="font-normal">
                  Resent
                </Badge>
              )}
            </span>
          </div>
        ),
      },
      {
        header: 'Recipients',
        accessorKey: 'recipients',
        cell: ({ row }) => (
          <span
            className="truncate text-muted-foreground"
            title={row.original.recipients.join(', ')}
          >
            {row.original.recipients.join(', ')}
          </span>
        ),
      },
      {
        header: 'Document',
        accessorKey: 'entityId',
        cell: ({ row }) => (
          <div className="flex min-w-0 flex-col gap-0.5">
            <EntityCell email={row.original} />
            {row.original.customerName && (
              <span className="text-xs text-muted-foreground">{row.original.customerName}</span>
            )}
          </div>
        ),
      },
      {
        header: 'Language',
        accessorKey: 'locale',
        cell: ({ row }) => (
          <span className="text-muted-foreground">
            {row.original.locale ? localeLabel(row.original.locale) : '—'}
          </span>
        ),
      },
      {
        id: 'actions',
        header: '',
        cell: ({ row }) => (
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            aria-label="Preview email"
            onClick={() => setPreviewId(row.original.id)}
          >
            <Eye size={14} />
          </Button>
        ),
      },
    ],
    []
  )

  return (
    <>
      <StandardTable
        columns={columns}
        data={emails}
        isLoading={query.isLoading}
        pagination={{ pageIndex: 0, pageSize: PAGE_SIZE }}
        setPagination={() => {}}
        totalCount={emails.length}
        emptyMessage={
          unavailable
            ? 'Sent emails are not available on this server yet.'
            : query.isError
              ? 'Couldn’t load sent emails.'
              : 'No emails sent yet.'
        }
      />

      <Pager cursors={cursors} setCursors={setCursors} nextCursor={nextCursor} />

      {previewId && (
        <EmailPreviewDialog
          sentEmailId={previewId}
          open
          onOpenChange={open => !open && setPreviewId(undefined)}
        />
      )}
    </>
  )
}

const SkippedList = ({ entityId, kind }: ListProps) => {
  const [cursors, setCursors] = useState<string[]>([])

  const currentCursor = cursors[cursors.length - 1]
  const query = useQuery(
    listSkippedEmails,
    entityId
      ? { invoicingEntityId: entityId, kind, cursor: currentCursor, limit: PAGE_SIZE }
      : skipToken,
    { retry: retryUnlessUnimplemented }
  )
  const emails = query.data?.emails ?? []
  const nextCursor = query.data?.nextCursor
  const unavailable = query.error?.code === Code.Unimplemented

  const columns = useMemo<ColumnDef<SkippedEmailSummary>[]>(
    () => [
      {
        header: 'Skipped',
        accessorKey: 'skippedAt',
        cell: ({ row }) => <When at={row.original.skippedAt} />,
      },
      {
        header: 'Email',
        accessorKey: 'kind',
        cell: ({ row }) => <span className="font-medium">{emailKindLabel(row.original.kind)}</span>,
      },
      {
        header: 'Document',
        accessorKey: 'entityId',
        cell: ({ row }) => <EntityCell email={row.original} />,
      },
      {
        header: 'Customer',
        accessorKey: 'customerName',
        cell: ({ row }) =>
          row.original.customerId ? (
            <EntityHoverCard
              entityType="customer"
              entityId={row.original.customerId}
              label={row.original.customerName ?? 'customer'}
            />
          ) : (
            <span className="text-muted-foreground">—</span>
          ),
      },
      {
        header: 'Reason',
        accessorKey: 'reason',
        cell: ({ row }) => (
          <span className="text-muted-foreground">{skipReason(row.original.reason) ?? '—'}</span>
        ),
      },
    ],
    []
  )

  return (
    <>
      <StandardTable
        columns={columns}
        data={emails}
        isLoading={query.isLoading}
        pagination={{ pageIndex: 0, pageSize: PAGE_SIZE }}
        setPagination={() => {}}
        totalCount={emails.length}
        emptyMessage={
          unavailable
            ? 'Skipped emails are not available on this server yet.'
            : query.isError
              ? 'Couldn’t load skipped emails.'
              : 'No skipped emails.'
        }
      />
      <Pager cursors={cursors} setCursors={setCursors} nextCursor={nextCursor} />
    </>
  )
}
