import { createConnectQueryKey } from '@connectrpc/connect-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  Card,
  CardContent,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ArrowLeftIcon, RefreshCwIcon, XCircleIcon } from 'lucide-react'
import { useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router-dom'
import { toast } from 'sonner'

import { getStatusConfig } from '@/features/admin/deadletter/statusConfig'
import { useMutation, useQuery } from '@/lib/connectrpc'
import {
  discardDeadLetter,
  getDeadLetter,
  requeueDeadLetter,
} from '@/rpc/admin/deadletter/v1/deadletter-DeadLetterService_connectquery'
import { DeadLetterStatus } from '@/rpc/admin/deadletter/v1/deadletter_pb'
import { parseAndFormatDateTime } from '@/utils/date'

export const DeadLetterDetail = () => {
  const { deadLetterId } = useParams<{ deadLetterId: string }>()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [showDiscardDialog, setShowDiscardDialog] = useState(false)

  const entryQuery = useQuery(getDeadLetter, { id: deadLetterId! })
  const entry = entryQuery.data?.entry

  const requeueMut = useMutation(requeueDeadLetter, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getDeadLetter, { id: deadLetterId }),
      })
      toast.success('Message requeued for reprocessing')
    },
    onError: error => {
      toast.error(`Failed to requeue: ${error.message}`)
    },
  })

  const discardMut = useMutation(discardDeadLetter, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(getDeadLetter, { id: deadLetterId }),
      })
      toast.success('Message discarded')
      setShowDiscardDialog(false)
    },
    onError: error => {
      toast.error(`Failed to discard: ${error.message}`)
    },
  })

  if (!entry) {
    return <div className="text-sm text-muted-foreground p-4">Loading...</div>
  }

  const { label, variant } = getStatusConfig(entry.status)
  const isPending = entry.status === DeadLetterStatus.PENDING

  let parsedMessage: string | undefined
  try {
    parsedMessage = entry.messageJson
      ? JSON.stringify(JSON.parse(entry.messageJson), null, 2)
      : undefined
  } catch {
    parsedMessage = entry.messageJson ?? undefined
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-4">
        <Button variant="ghost" size="sm" onClick={() => navigate(-1)}>
          <ArrowLeftIcon size={16} className="mr-1" />
          Back
        </Button>
        <h1 className="text-lg font-semibold">Dead Letter Detail</h1>
        <Badge variant={variant}>{label}</Badge>
      </div>

      {isPending && (
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => requeueMut.mutate({ id: entry.id })}
            disabled={requeueMut.isPending}
          >
            <RefreshCwIcon size={14} className="mr-1" />
            Requeue
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setShowDiscardDialog(true)}
            disabled={discardMut.isPending}
          >
            <XCircleIcon size={14} className="mr-1" />
            Discard
          </Button>
        </div>
      )}

      {entry.requeuedPgmqMsgId != null && entry.requeuedPgmqMsgId !== 0n && (
        <div className="flex items-center gap-2 p-3 bg-muted rounded-md text-sm">
          <RefreshCwIcon size={14} className="text-muted-foreground" />
          <span>
            Requeued as PGMQ message <span className="font-mono">{String(entry.requeuedPgmqMsgId)}</span>
            {entry.requeuedDeadLetterId && (
              <>
                {' — '}
                <Link to={`../${entry.requeuedDeadLetterId}`} className="text-primary underline">
                  failed again (view)
                </Link>
              </>
            )}
          </span>
        </div>
      )}

      <div className="grid grid-cols-2 gap-4 max-w-2xl">
        <InfoRow label="Queue" value={entry.queue} mono />
        <InfoRow label="PGMQ Message ID" value={String(entry.pgmqMsgId)} mono />
        {entry.organizationName && (
          <InfoRow
            label="Organization"
            value={`${entry.organizationName} (${entry.organizationSlug})`}
          />
        )}
        {entry.tenantName && (
          <InfoRow
            label="Tenant"
            value={`${entry.tenantName} (${entry.tenantSlug})`}
          />
        )}
        <InfoRow label="Read Count" value={String(entry.readCount)} />
        <InfoRow label="Enqueued At" value={parseAndFormatDateTime(entry.enqueuedAt)} />
        <InfoRow label="Dead-lettered At" value={parseAndFormatDateTime(entry.deadLetteredAt)} />
        {entry.resolvedAt && (
          <InfoRow label="Resolved At" value={parseAndFormatDateTime(entry.resolvedAt)} />
        )}
      </div>

      {entry.lastError && (
        <Card>
          <CardContent className="pt-4">
            <h3 className="text-sm font-medium mb-2">Last Error</h3>
            <pre className="text-xs bg-muted p-3 rounded-md overflow-x-auto whitespace-pre-wrap break-words">
              {entry.lastError}
            </pre>
          </CardContent>
        </Card>
      )}

      {parsedMessage && (
        <Card>
          <CardContent className="pt-4">
            <h3 className="text-sm font-medium mb-2">Message Payload</h3>
            <pre className="text-xs bg-muted p-3 rounded-md overflow-x-auto max-h-[400px] overflow-y-auto">
              {parsedMessage}
            </pre>
          </CardContent>
        </Card>
      )}

      {entry.headersJson && (
        <Card>
          <CardContent className="pt-4">
            <h3 className="text-sm font-medium mb-2">Headers</h3>
            <pre className="text-xs bg-muted p-3 rounded-md overflow-x-auto">
              {(() => {
                try {
                  return JSON.stringify(JSON.parse(entry.headersJson), null, 2)
                } catch {
                  return entry.headersJson
                }
              })()}
            </pre>
          </CardContent>
        </Card>
      )}

      <AlertDialog open={showDiscardDialog} onOpenChange={setShowDiscardDialog}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard this message?</AlertDialogTitle>
            <AlertDialogDescription>
              This marks the message as discarded. It will not be reprocessed.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={() => discardMut.mutate({ id: entry.id })}>
              Discard
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

const InfoRow = ({ label, value, mono }: { label: string; value: string; mono?: boolean }) => (
  <div>
    <dt className="text-xs text-muted-foreground">{label}</dt>
    <dd className={`text-sm ${mono ? 'font-mono' : ''}`}>{value}</dd>
  </div>
)
