import {
  Alert,
  Button,
  DialogDescription,
  DialogTitle,
  Modal,
  Spinner,
  cn,
} from '@md/ui'
import { format, formatDistanceToNow, isValid } from 'date-fns'
import { RefreshCw, X } from 'lucide-react'
import { ReactNode, useCallback, useEffect, useRef } from 'react'
import { useNavigate, useParams } from 'react-router-dom'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { useDismissRouteModal } from '@/hooks/useDismissRouteModal'
import { useQuery } from '@/lib/connectrpc'
import {
  getConnectorUsage,
  listConnectors,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import {
  Connector,
  ConnectorStatusEnum,
  ConnectorUsage,
} from '@/rpc/api/connectors/v1/models_pb'

import { AccountDetails } from './AccountDetails'
import { useConnectorLifecycle, useConnectorPaths } from './ConnectorLifecycle'
import {
  ConnectorStatusBadge,
  CredentialsErrorBadge,
  MethodChips,
  joinNames,
  methodChips,
} from './connectors'
import { PaymentProviderDefinition, paymentProvider } from './providers'

/** Route modal at `manage-connector/:connectorId`: account, methods, usage and lifecycle. */
export const ManageConnectorModal = () => {
  const { connectorId } = useParams<{ connectorId: string }>()
  const dismiss = useDismissRouteModal()
  const connectorsQuery = useQuery(listConnectors, {})
  const connector = connectorsQuery.data?.connectors.find(c => c.id === connectorId)
  const definition = paymentProvider(connector?.provider)

  // Once only: a delete both closes the modal and drops the connector from the list.
  const closed = useRef(false)
  const closeModal = useCallback(() => {
    if (closed.current) return
    closed.current = true
    dismiss()
  }, [dismiss])

  useEffect(() => {
    if (connectorsQuery.isSuccess && (!connector || !definition)) closeModal()
  }, [connectorsQuery.isSuccess, connector, definition, closeModal])

  if (!connector || !definition) return null
  return <ManageConnector connector={connector} definition={definition} onClose={closeModal} />
}

const ManageConnector = ({
  connector,
  definition,
  onClose,
}: {
  connector: Connector
  definition: PaymentProviderDefinition
  onClose: () => void
}) => {
  const navigate = useNavigate()
  const paths = useConnectorPaths()
  const { Logo, name } = definition
  const lifecycle = useConnectorLifecycle({ onDeleted: onClose })
  const usageQuery = useQuery(getConnectorUsage, { id: connector.id })
  const usage = usageQuery.data?.usage

  const isDisabled = connector.status === ConnectorStatusEnum.DISABLED
  const isDraft = connector.status === ConnectorStatusEnum.DRAFT
  const enableError = lifecycle.enableErrorFor(connector.id)
  const routedCount = usage
    ? new Set([...usage.routedCard, ...usage.routedDirectDebit].map(e => e.id)).size
    : 0

  const updateCredentials = () => navigate(paths.credentials(connector.id))
  const checkedAt = parseDate(connector.account?.checkedAt)
  const createdAt = parseDate(connector.createdAt)

  return (
    <>
      <Modal
        visible
        hideFooter
        size="large"
        onCancel={onClose}
        header={
          <div className="flex items-start justify-between gap-3">
            <div className="space-y-1">
              <DialogTitle className="flex items-center gap-2 text-md">
                <Logo className="h-6 w-6 text-foreground" />
                <span>{connector.alias}</span>
                <ConnectorStatusBadge status={connector.status} />
                <CredentialsErrorBadge connector={connector} />
              </DialogTitle>
              <DialogDescription className="text-sm">
                {name}
                {createdAt && ` · created ${format(createdAt, 'd MMM yyyy')}`}
              </DialogDescription>
            </div>
            <Button size="icon" variant="ghost" title="Close" aria-label="Close" onClick={onClose}>
              <X size={16} />
            </Button>
          </div>
        }
      >
        <Modal.Content>
          <div className="space-y-6 py-5">
            {isDraft && (
              <Alert variant="warning">
                <div className="flex items-center justify-between gap-3">
                  <span>This connection isn&apos;t set up yet.</span>
                  <Button size="sm" onClick={() => navigate(paths.resume(connector), { replace: true })}>
                    Resume setup
                  </Button>
                </div>
              </Alert>
            )}

            {!isDraft && (
              <ConnectionHealth
                connector={connector}
                providerName={name}
                checking={lifecycle.isChecking(connector.id)}
                onCheck={() => lifecycle.check(connector)}
                onUpdateCredentials={updateCredentials}
              />
            )}

            {isDisabled && (
              <div className="space-y-3 rounded-lg border border-border bg-muted/40 p-4 text-sm">
                <p>
                  This connection is disabled. No new payments are collected through it.
                  {routedCount > 0 &&
                    ` ${routedCount} invoicing ${routedCount === 1 ? 'entity still routes' : 'entities still route'} to it.`}
                </p>
                <div className="flex gap-2">
                  <Button
                    size="sm"
                    onClick={() => lifecycle.enable(connector)}
                    hasIcon={lifecycle.isEnabling(connector.id)}
                    disabled={lifecycle.isEnabling(connector.id)}
                  >
                    {lifecycle.isEnabling(connector.id) && <Spinner />}
                    Enable
                  </Button>
                </div>
                <p className="text-xs text-muted-foreground">
                  Enabling re-checks the credentials with {name} first.
                </p>
                {enableError && <Alert variant="destructive">{enableError}</Alert>}
              </div>
            )}

            <Section
              title="Account"
              action={
                // While the key is rejected, the health row above carries this action.
                !isDraft &&
                !connector.credentialsError &&
                definition.connect.credentialFields.length > 0 && (
                  <Button size="sm" variant="secondary" onClick={updateCredentials}>
                    Update credentials
                  </Button>
                )
              }
            >
              <AccountDetails connector={connector} definition={definition} />
            </Section>

            <Section title="Payment methods">
              <MethodChips chips={methodChips(connector)} />
              {!checkedAt && (
                <p className="mt-2 text-xs text-muted-foreground">
                  Showing what the integration supports until the connection is checked.
                </p>
              )}
            </Section>

            {connector.webhookUrl && (
              <Section title="Webhook">
                <CopyToClipboardButton
                  text={connector.webhookUrl}
                  buttonClassName="max-w-full h-auto items-start text-left whitespace-normal"
                  className="whitespace-normal break-all"
                />
                <p className="mt-2 text-xs text-muted-foreground">
                  {!connector.requiresWebhookSecret
                    ? 'Nothing to configure.'
                    : connector.hasWebhookSecret
                      ? 'Signing secret configured.'
                      : 'No signing secret configured: webhooks from the provider are rejected.'}
                </p>
              </Section>
            )}

            <Section title="Used by">
              {usageQuery.isLoading ? (
                <Spinner />
              ) : usage ? (
                <UsageSummary usage={usage} />
              ) : (
                <p className="text-xs text-muted-foreground">Usage couldn&apos;t be loaded.</p>
              )}
            </Section>

            {!isDraft && (
              <div className="divide-y divide-destructive/20 rounded-lg border border-destructive/30">
                {connector.status === ConnectorStatusEnum.ACTIVE && (
                  <DangerRow
                    title="Disable connection"
                    description="Stop new charges and new payment methods. Payments already in progress still settle."
                    action={
                      <Button
                        size="sm"
                        variant="destructiveGhost"
                        onClick={() => lifecycle.requestDisable(connector)}
                      >
                        Disable
                      </Button>
                    }
                  />
                )}
                <DangerRow
                  title="Delete connection"
                  description="Removes it from Meteroid and wipes its credentials. Payment history is kept."
                  action={
                    <Button
                      size="sm"
                      variant="destructive"
                      onClick={() => lifecycle.requestDelete(connector)}
                    >
                      Delete…
                    </Button>
                  }
                />
              </div>
            )}
          </div>
        </Modal.Content>
      </Modal>
      {lifecycle.dialogs}
    </>
  )
}

const parseDate = (value: string | undefined) => {
  if (!value) return undefined
  const date = new Date(value)
  return isValid(date) ? date : undefined
}

const Section = ({
  title,
  action,
  children,
}: {
  title: string
  action?: ReactNode
  children: ReactNode
}) => (
  <section className="space-y-2">
    <div className="flex items-center justify-between gap-2">
      <h4 className="text-sm font-medium">{title}</h4>
      {action}
    </div>
    {children}
  </section>
)

const DangerRow = ({
  title,
  description,
  action,
}: {
  title: string
  description: string
  action: ReactNode
}) => (
  <div className="flex items-center justify-between gap-4 p-4">
    <div className="space-y-0.5">
      <p className="text-sm font-medium">{title}</p>
      <p className="text-xs text-muted-foreground">{description}</p>
    </div>
    {action}
  </div>
)

const UsageSummary = ({ usage }: { usage: ConnectorUsage }) => {
  const routing = [
    usage.routedCard.length > 0 &&
      `Card routing for ${joinNames(usage.routedCard.map(e => e.name))}`,
    usage.routedDirectDebit.length > 0 &&
      `Direct debit routing for ${joinNames(usage.routedDirectDebit.map(e => e.name))}`,
  ].filter(Boolean)
  const counts = [
    `${usage.customersWithSavedMethods} ${usage.customersWithSavedMethods === 1 ? 'customer' : 'customers'} with saved payment methods`,
    `${usage.activeSubscriptions} active ${usage.activeSubscriptions === 1 ? 'subscription' : 'subscriptions'}`,
  ]
  return (
    <div className="space-y-1 text-xs text-muted-foreground">
      <p className="text-foreground">
        {routing.length > 0 ? routing.join(' · ') : 'Not routed for any invoicing entity.'}
      </p>
      <p>{counts.join(' · ')}</p>
    </div>
  )
}

/**
 * One place for the connection's health: whether the provider still accepts the stored
 * credentials, when that was last checked, and the action that fits.
 */
const ConnectionHealth = ({
  connector,
  providerName,
  checking,
  onCheck,
  onUpdateCredentials,
}: {
  connector: Connector
  providerName: string
  checking: boolean
  onCheck: () => void
  onUpdateCredentials: () => void
}) => {
  const error = connector.credentialsError
  const errorAt = parseDate(error?.at)
  const checkedAt = parseDate(connector.account?.checkedAt)

  const summary = error
    ? `${providerName} rejected the stored credentials${errorAt ? ` on ${format(errorAt, 'd MMM yyyy')}` : ''}`
    : checkedAt
      ? `Connection OK · checked ${formatDistanceToNow(checkedAt, { addSuffix: true })}`
      : 'Not checked yet'

  return (
    <div
      className={cn(
        'flex items-center justify-between gap-3 rounded-lg border px-4 py-3 text-sm',
        error ? 'border-destructive/40 bg-destructive/5' : 'border-border'
      )}
    >
      <div className="min-w-0 space-y-0.5">
        <p className="flex items-center gap-2">
          <span
            className={cn(
              'h-2 w-2 shrink-0 rounded-full',
              error ? 'bg-destructive' : checkedAt ? 'bg-success' : 'bg-muted-foreground'
            )}
          />
          {summary}
        </p>
        {error && (
          <p className="break-words pl-4 text-xs text-muted-foreground">{error.message}</p>
        )}
      </div>
      <div className="flex shrink-0 gap-2">
        {error && (
          <Button size="sm" onClick={onUpdateCredentials}>
            Update credentials
          </Button>
        )}
        <Button
          size="sm"
          variant={error ? 'ghost' : 'secondary'}
          hasIcon
          onClick={onCheck}
          disabled={checking}
        >
          <RefreshCw size={14} className={checking ? 'animate-spin' : undefined} />
          {error ? 'Check again' : 'Check connection'}
        </Button>
      </div>
    </div>
  )
}
