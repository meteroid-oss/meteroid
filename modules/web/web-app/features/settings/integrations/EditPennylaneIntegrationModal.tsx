import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Alert,
  Badge,
  Button,
  DialogDescription,
  DialogTitle,
  Form,
  Modal,
  SelectFormField,
  SelectItem,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ExternalLinkIcon, RefreshCwIcon, Settings2Icon } from 'lucide-react'
import { useEffect } from 'react'
import { Link, useParams } from 'react-router'
import { toast } from 'sonner'
import { z } from 'zod'

import { InlineLoading } from '@/components/Loading'
import {
  ALL_INVOICING_ENTITIES,
  pennylaneIntegrationSchema,
} from '@/features/settings/integrations/schemas'
import { useBasePath } from '@/hooks/useBasePath'
import { useDismissRouteModal } from '@/hooks/useDismissRouteModal'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  listConnectorSyncIssues,
  listConnectors,
  retryConnectorSyncIssues,
  updatePennylaneConnector,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorSyncIssue, SyncResourceType } from '@/rpc/api/connectors/v1/models_pb'
import { listInvoicingEntities } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'

const RESOURCES: Record<SyncResourceType, { label: string; path: string }> = {
  [SyncResourceType.SYNC_RESOURCE_CUSTOMER]: { label: 'Customer', path: 'customers' },
  [SyncResourceType.SYNC_RESOURCE_INVOICE]: { label: 'Invoice', path: 'invoices' },
  [SyncResourceType.SYNC_RESOURCE_CREDIT_NOTE]: { label: 'Credit note', path: 'credit-notes' },
}

export const EditPennylaneIntegrationModal = () => {
  const closeModal = useDismissRouteModal()
  const { connectionId } = useParams()
  const queryClient = useQueryClient()

  const connectorsQuery = useQuery(listConnectors, {})
  const entitiesQuery = useQuery(listInvoicingEntities)
  const issuesQuery = useQuery(
    listConnectorSyncIssues,
    { id: connectionId ?? '' },
    { enabled: !!connectionId }
  )

  const connection = connectorsQuery.data?.connectors.find(conn => conn.id === connectionId)
  const pennylane = connection?.data?.data?.case === 'pennylane' ? connection.data.data.value : undefined

  const methods = useZodForm({
    mode: 'onChange',
    schema: pennylaneIntegrationSchema,
    defaultValues: { invoicingEntityId: ALL_INVOICING_ENTITIES },
  })

  const loaded = !!pennylane
  const scope = pennylane?.invoicingEntityId
  useEffect(() => {
    if (loaded) {
      methods.reset({ invoicingEntityId: scope ?? ALL_INVOICING_ENTITIES })
    }
  }, [loaded, scope, methods])

  const updateMutation = useMutation(updatePennylaneConnector, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listConnectors, cardinality: 'finite' }),
      })
      toast.success('Connection updated!')
    },
    onError: error => toast.error(`Connection update failure. ${error.message}`),
  })

  const retryMutation = useMutation(retryConnectorSyncIssues, {
    onSuccess: ({ requeued }) => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listConnectorSyncIssues,
          cardinality: 'finite',
        }),
      })
      toast.success(`${requeued} document${requeued === 1 ? '' : 's'} sent to Pennylane again`)
    },
    onError: error => toast.error(`Retry failure. ${error.message}`),
  })

  const onSubmit = async (data: z.infer<typeof pennylaneIntegrationSchema>) => {
    await updateMutation.mutateAsync({
      id: connectionId,
      invoicingEntityId:
        data.invoicingEntityId === ALL_INVOICING_ENTITIES ? undefined : data.invoicingEntityId,
    })
    closeModal()
  }

  if (!connection || !pennylane) {
    return <InlineLoading />
  }

  const entities = entitiesQuery.data?.entities ?? []
  const issues = issuesQuery.data?.issues ?? []

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <Settings2Icon className="w-6 h-6 text-blue" />
            <span>Pennylane</span>
          </DialogTitle>
          <DialogDescription>
            Customers, invoices and credit notes are imported into{' '}
            <a
              className="inline-flex items-center gap-1 underline"
              href={`https://app.pennylane.com/companies/${pennylane.externalCompanyId}`}
              target="_blank"
              rel="noreferrer"
            >
              your Pennylane company <ExternalLinkIcon className="h-3 w-3" />
            </a>{' '}
            as they are issued, and invoices are reconciled as they are paid.
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={false}
      onCancel={() => closeModal()}
      onConfirm={methods.handleSubmit(onSubmit)}
    >
      <Modal.Content>
        <div className="space-y-6">
          {connection.credentialsError && (
            <Alert variant="destructive">{connection.credentialsError.message}</Alert>
          )}

          {entities.length > 1 && (
            <Form {...methods}>
              <form autoComplete="off">
                <SelectFormField
                  name="invoicingEntityId"
                  control={methods.control}
                  label="Invoicing entity"
                  labelTooltip="A Pennylane company keeps the books of one legal entity: only the documents of this entity are synced."
                >
                  <SelectItem value={ALL_INVOICING_ENTITIES}>All entities</SelectItem>
                  {entities.map(entity => (
                    <SelectItem key={entity.id} value={entity.id}>
                      {entity.legalName}
                    </SelectItem>
                  ))}
                </SelectFormField>
              </form>
            </Form>
          )}

          <SyncIssues
            issues={issues}
            loading={issuesQuery.isLoading}
            failed={issuesQuery.isError}
            retrying={retryMutation.isPending}
            onRetry={() => retryMutation.mutate({ id: connectionId })}
          />
        </div>
      </Modal.Content>
    </Modal>
  )
}

interface SyncIssuesProps {
  issues: ConnectorSyncIssue[]
  loading: boolean
  failed: boolean
  retrying: boolean
  onRetry: () => void
}

const SyncIssues = ({ issues, loading, failed, retrying, onRetry }: SyncIssuesProps) => {
  const basePath = useBasePath()

  if (loading) {
    return <InlineLoading />
  }
  if (failed) {
    return <Alert variant="warning">The sync issues could not be loaded.</Alert>
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <div>
          <div className="text-sm font-medium">Sync issues</div>
          <div className="text-xs text-muted-foreground">
            {issues.length === 0
              ? 'Every document reached Pennylane.'
              : 'These documents did not reach Pennylane. Fix what is reported, then retry.'}
          </div>
        </div>
        {issues.length > 0 && (
          <Button size="sm" variant="outline" disabled={retrying} onClick={onRetry}>
            <RefreshCwIcon className="mr-2 h-3 w-3" />
            Retry all
          </Button>
        )}
      </div>

      {issues.length > 0 && (
        <ul className="max-h-80 divide-y overflow-y-auto rounded-md border">
          {issues.map(issue => {
            const resource = RESOURCES[issue.resourceType]
            return (
              <li key={issue.id} className="space-y-1 px-3 py-2 text-sm">
                <div className="flex items-center justify-between gap-2">
                  <Link
                    className="flex items-center gap-2 font-medium underline-offset-2 hover:underline"
                    to={`${basePath}/${resource.path}/${issue.resourceId}`}
                  >
                    <Badge variant="outline">{resource.label}</Badge>
                    <span className="font-mono text-xs">{issue.resourceId}</span>
                  </Link>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {parseAndFormatDateTime(issue.lastSeenAt)}
                    {issue.occurrences > 1 && ` · ${issue.occurrences} attempts`}
                  </span>
                </div>
                <div className="text-muted-foreground">{issue.message}</div>
              </li>
            )
          })}
        </ul>
      )}
    </div>
  )
}
