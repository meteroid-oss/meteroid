import { Code, ConnectError } from '@connectrpc/connect'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Input, Modal, Spinner } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ReactNode, useCallback, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import {
  deleteConnector,
  disableConnector,
  enableConnector,
  getConnectorUsage,
  listConnectors,
  checkConnector,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { Connector, ConnectorStatusEnum } from '@/rpc/api/connectors/v1/models_pb'
import { getInvoicingEntityProviders } from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import { joinNames, userFacingError } from './connectors'
import { paymentProvider } from './providers'

/** Absolute links to the connector route modals, usable from any settings screen. */
export const useConnectorPaths = () => {
  const basePath = useBasePath()
  return {
    manage: (id: string) => `${basePath}/settings/integrations/manage-connector/${id}`,
    credentials: (id: string) => `${basePath}/settings/integrations/manage-connector/${id}/credentials`,
    resume: (connector: Connector) =>
      `${basePath}/settings/integrations/connect-payment-provider/${paymentProvider(connector.provider)?.key}?draft=${connector.id}`,
  }
}

/** Refreshes everything a lifecycle change can affect: the list, usage, and entity routing. */
export const useInvalidateConnectors = () => {
  const queryClient = useQueryClient()
  return useCallback(() => {
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: listConnectors, cardinality: undefined }),
    })
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: getConnectorUsage, cardinality: undefined }),
    })
    // Removed, not invalidated: the payment-methods form seeds its defaults from this query on
    // mount only, so a background refetch wouldn't reach it.
    queryClient.removeQueries({
      queryKey: createConnectQueryKey({
        schema: getInvoicingEntityProviders,
        cardinality: undefined,
      }),
    })
  }, [queryClient])
}

/**
 * Disable / enable / delete / check / discard for a connection, with their dialogs. Render
 * `dialogs` once in the host component.
 */
export const useConnectorLifecycle = ({ onDeleted }: { onDeleted?: () => void } = {}) => {
  const navigate = useNavigate()
  const paths = useConnectorPaths()
  const invalidate = useInvalidateConnectors()
  const showConfirmationModal = useConfirmationModal()

  const [disableTarget, setDisableTarget] = useState<Connector>()
  const [deleteTarget, setDeleteTarget] = useState<Connector>()
  const [enableError, setEnableError] = useState<{ id: string; message: string }>()

  const enableMut = useMutation(enableConnector, { onSuccess: invalidate })
  // Settled, not success: a rejected key flags the connection, which the list must show.
  const checkMut = useMutation(checkConnector, { onSettled: invalidate })
  const discardMut = useMutation(deleteConnector, { onSuccess: invalidate })

  const enable = async (connector: Connector) => {
    setEnableError(undefined)
    try {
      await enableMut.mutateAsync({ id: connector.id })
      toast.success(`${connector.alias} enabled`)
    } catch (error) {
      // Enabling re-checks the credentials; a rejection means they need updating.
      const message = userFacingError(
        error,
        `Couldn't enable ${connector.alias}.`,
        paymentProvider(connector.provider)?.name
      )
      setEnableError({ id: connector.id, message })
      toast.error(message, {
        action: {
          label: 'Update credentials',
          onClick: () => navigate(paths.credentials(connector.id)),
        },
      })
    }
  }

  /** Asks the provider whether the stored credentials still work, and reloads the account. */
  const check = async (connector: Connector) => {
    const providerName = paymentProvider(connector.provider)?.name ?? 'the provider'
    try {
      await checkMut.mutateAsync({ id: connector.id })
      toast.success(`${connector.alias} checked`)
    } catch (error) {
      toast.error(`Couldn't check ${connector.alias}`, {
        description: userFacingError(error, `${providerName} couldn't be reached.`, providerName),
        action: {
          label: 'Update credentials',
          onClick: () => navigate(paths.credentials(connector.id)),
        },
      })
    }
  }



  const discardDraft = (connector: Connector, onDone?: () => void) =>
    showConfirmationModal(
      () =>
        discardMut.mutate(
          { id: connector.id },
          {
            onSuccess: () => {
              toast.success('Draft discarded')
              onDone?.()
            },
            onError: error => toast.error(userFacingError(error, "Couldn't discard the draft.")),
          }
        ),
      {
        header: `Discard ${connector.alias}?`,
        message: 'The unfinished connection and anything entered so far are removed.',
      },
      { buttonLabel: 'Discard draft' }
    )

  const dialogs: ReactNode = (
    <>
      {disableTarget && (
        <DisableConnectorDialog
          connector={disableTarget}
          onClose={() => setDisableTarget(undefined)}
        />
      )}
      {deleteTarget && (
        <DeleteConnectorDialog
          connector={deleteTarget}
          onClose={() => setDeleteTarget(undefined)}
          onDisableInstead={() => {
            setDeleteTarget(undefined)
            setDisableTarget(deleteTarget)
          }}
          onDeleted={() => {
            setDeleteTarget(undefined)
            onDeleted?.()
          }}
        />
      )}
    </>
  )

  return {
    requestDisable: setDisableTarget,
    requestDelete: setDeleteTarget,
    enable,
    check,
    discardDraft,
    isEnabling: (id: string) => enableMut.isPending && enableMut.variables?.id === id,
    isChecking: (id: string) => checkMut.isPending && checkMut.variables?.id === id,
    enableErrorFor: (id: string) => (enableError?.id === id ? enableError.message : undefined),
    dialogs,
  }
}

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`

const DisableConnectorDialog = ({
  connector,
  onClose,
}: {
  connector: Connector
  onClose: () => void
}) => {
  const invalidate = useInvalidateConnectors()
  const usageQuery = useQuery(getConnectorUsage, { id: connector.id })
  const usage = usageQuery.data?.usage
  const disableMut = useMutation(disableConnector, { onSuccess: invalidate })

  const routedEntities = [
    ...new Set([...(usage?.routedCard ?? []), ...(usage?.routedDirectDebit ?? [])].map(e => e.name)),
  ]

  const onConfirm = async () => {
    try {
      await disableMut.mutateAsync({ id: connector.id })
      toast.success(`${connector.alias} disabled`)
      onClose()
    } catch (error) {
      toast.error(userFacingError(error, `Couldn't disable ${connector.alias}.`))
    }
  }

  return (
    <Modal
      visible
      size="medium"
      header={`Disable ${connector.alias}?`}
      onCancel={onClose}
      onConfirm={onConfirm}
      confirmText="Disable connection"
      loading={disableMut.isPending}
      confirmDisabled={usageQuery.isLoading}
    >
      <Modal.Content>
        <div className="py-4 text-sm">
          {usageQuery.isLoading ? (
            <Spinner />
          ) : (
            <>
              <p className="mb-2 font-medium">While disabled:</p>
              <ul className="list-disc space-y-1.5 pl-5 text-muted-foreground">
                {!!usage?.activeSubscriptions && (
                  <li>
                    {plural(usage.activeSubscriptions, "subscription won't", "subscriptions won't")}{' '}
                    be charged automatically. Their invoices stay open for manual payment.
                  </li>
                )}
                <li>Customers can&apos;t add payment methods through it.</li>
                {routedEntities.length > 0 && (
                  <li>
                    {joinNames(routedEntities)} {routedEntities.length === 1 ? 'keeps' : 'keep'} it
                    as {routedEntities.length === 1 ? 'its' : 'their'} routed provider, marked as
                    disabled.
                  </li>
                )}
                <li>
                  {usage?.inFlightTransactions
                    ? `${plural(usage.inFlightTransactions, 'payment', 'payments')} already in progress will still settle. `
                    : ''}
                  Refunds still work.
                </li>
              </ul>
            </>
          )}
        </div>
      </Modal.Content>
    </Modal>
  )
}

const DeleteConnectorDialog = ({
  connector,
  onClose,
  onDisableInstead,
  onDeleted,
}: {
  connector: Connector
  onClose: () => void
  onDisableInstead: () => void
  onDeleted: () => void
}) => {
  const invalidate = useInvalidateConnectors()
  const usageQuery = useQuery(getConnectorUsage, { id: connector.id })
  const usage = usageQuery.data?.usage
  const deleteMut = useMutation(deleteConnector, { onSuccess: invalidate })
  const [typed, setTyped] = useState('')
  // A payment can start between loading the usage and confirming; the server refuses then.
  const [serverBlock, setServerBlock] = useState<string>()

  const provider = paymentProvider(connector.provider)
  const inFlight = usage?.inFlightTransactions ?? 0
  const blocked = inFlight > 0 || !!serverBlock
  const routedEntities = [
    ...new Set([...(usage?.routedCard ?? []), ...(usage?.routedDirectDebit ?? [])].map(e => e.name)),
  ]

  const onConfirm = async () => {
    try {
      await deleteMut.mutateAsync({ id: connector.id })
      toast.success(`${connector.alias} deleted`)
      onDeleted()
    } catch (error) {
      const err = ConnectError.from(error)
      if (err.code === Code.FailedPrecondition) {
        setServerBlock(err.rawMessage)
        usageQuery.refetch()
      } else {
        toast.error(userFacingError(error, `Couldn't delete ${connector.alias}.`))
      }
    }
  }

  if (usageQuery.isLoading) {
    return (
      <Modal visible size="medium" header={`Delete ${connector.alias}?`} onCancel={onClose} hideFooter>
        <Modal.Content>
          <div className="flex justify-center py-6">
            <Spinner />
          </div>
        </Modal.Content>
      </Modal>
    )
  }

  if (blocked) {
    const canDisable = connector.status === ConnectorStatusEnum.ACTIVE
    return (
      <Modal
        visible
        size="medium"
        header={`Can't delete ${connector.alias} yet`}
        onCancel={onClose}
        customFooter={
          <div className="flex w-full justify-end space-x-2">
            <Button variant="secondary" size="sm" onClick={onClose}>
              Close
            </Button>
            {canDisable && (
              <Button size="sm" onClick={onDisableInstead}>
                Disable instead
              </Button>
            )}
          </div>
        }
      >
        <Modal.Content>
          <div className="space-y-3 py-4 text-sm text-muted-foreground">
            <p>
              {inFlight > 0
                ? `${plural(inFlight, 'payment is', 'payments are')} still in progress through this connection.`
                : serverBlock}{' '}
              Deleting it now would stop us from receiving their outcome.
            </p>
            <p>
              {canDisable ? 'Disable it now to stop new charges, then delete' : 'Delete it'} once
              they&apos;ve settled. Direct debits usually take up to 5 business days.
            </p>
          </div>
        </Modal.Content>
      </Modal>
    )
  }

  return (
    <Modal
      visible
      size="medium"
      header={`Delete ${connector.alias}?`}
      onCancel={onClose}
      customFooter={
        <div className="flex w-full justify-end space-x-2">
          <Button variant="secondary" size="sm" onClick={onClose} disabled={deleteMut.isPending}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            size="sm"
            onClick={onConfirm}
            hasIcon={deleteMut.isPending}
            disabled={typed !== connector.alias || deleteMut.isPending}
          >
            {deleteMut.isPending && <Spinner />}
            Delete connection
          </Button>
        </div>
      }
    >
      <Modal.Content>
        <div className="space-y-4 py-4 text-sm">
          <ul className="list-disc space-y-1.5 pl-5 text-muted-foreground">
            <li>
              Removed from Integrations
              {routedEntities.length > 0 ? ` and from routing for ${joinNames(routedEntities)}` : ''}
              .
            </li>
            <li>
              Stored credentials are wiped
              {provider?.connect.webhook?.mode === 'auto'
                ? ` and the webhook endpoint we created in ${provider.name} is removed`
                : ''}
              .
            </li>
            {!!usage?.customersWithSavedMethods && (
              <li>
                {plural(usage.customersWithSavedMethods, "customer's", "customers'")} saved payment
                methods from this connection are hidden.
              </li>
            )}
            <li>Invoices, payments and refunds keep their history.</li>
          </ul>
          <div className="space-y-1.5">
            <label htmlFor="confirm-alias" className="text-xs text-muted-foreground">
              Type <span className="font-mono text-foreground">{connector.alias}</span> to confirm
            </label>
            <Input
              id="confirm-alias"
              value={typed}
              onChange={e => setTyped(e.target.value)}
              autoComplete="off"
              onKeyDown={e => {
                if (e.key === 'Enter' && typed === connector.alias) onConfirm()
              }}
            />
          </div>
        </div>
      </Modal.Content>
    </Modal>
  )
}
