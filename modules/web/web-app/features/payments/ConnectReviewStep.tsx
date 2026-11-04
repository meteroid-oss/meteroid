import { useMutation } from '@connectrpc/connect-query'
import { Alert, Button, Checkbox, Spinner, cn } from '@md/ui'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { useQuery } from '@/lib/connectrpc'
import { activateConnector } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { Connector } from '@/rpc/api/connectors/v1/models_pb'
import {
  getInvoicingEntityProviders,
  updateInvoicingEntityProviders,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

import { AccountDetails } from './AccountDetails'
import { useInvalidateConnectors } from './ConnectorLifecycle'
import { WizardFooter } from './WizardFooter'
import { RAIL_LABELS, railStatus, userFacingError } from './connectors'
import { PaymentProviderDefinition, PaymentRail } from './providers'

interface Props {
  connector: Connector
  definition: PaymentProviderDefinition
  /** How the webhook got configured, for the summary; absent when the provider has none. */
  webhookNote?: string
  onBack: () => void
  /** Called once active (routing applied or skipped). */
  onDone: () => void
}

/**
 * Last wizard step: review the account, pick routing for the default invoicing entity, activate.
 * Activation runs first, then the routing patch, which the backend only accepts for active
 * connectors.
 *
 * Routing is merged with the entity's existing providers so an unchecked rail is never wiped:
 * the backend patch overwrites all three provider slots at once.
 */
export const ConnectReviewStep = ({ connector, definition, webhookNote, onBack, onDone }: Props) => {
  const invalidate = useInvalidateConnectors()
  const { defaultEntity, entities } = useInvoicingEntity()

  const entityId = defaultEntity?.id
  const providersQuery = useQuery(
    getInvoicingEntityProviders,
    { id: entityId! },
    { enabled: !!entityId }
  )

  const currentCard = providersQuery.data?.cardProvider
  const currentDebit = providersQuery.data?.directDebitProvider
  const currentBankId = providersQuery.data?.bankAccount?.id

  const [selected, setSelected] = useState<Record<PaymentRail, boolean>>({
    card: false,
    directDebit: false,
  })
  const [initialized, setInitialized] = useState(false)
  const [error, setError] = useState<string>()

  const rails = (['card', 'directDebit'] as const)
    .map(rail => ({ rail, status: railStatus(connector, rail) }))
    .filter(r => r.status.supported)

  // Default each enabled rail ON when nothing is routed there yet, so the common "first provider"
  // case is a single confirmation. Leave it OFF (opt-in) when the rail already points somewhere,
  // to avoid silently repointing existing routing.
  useEffect(() => {
    if (initialized || !entityId || !providersQuery.isSuccess) return
    setSelected({
      card: railStatus(connector, 'card').enabled && !currentCard,
      directDebit: railStatus(connector, 'directDebit').enabled && !currentDebit,
    })
    setInitialized(true)
  }, [initialized, entityId, providersQuery.isSuccess, connector, currentCard, currentDebit])

  const activateMut = useMutation(activateConnector, { onSuccess: invalidate })
  const routingMut = useMutation(updateInvoicingEntityProviders, { onSuccess: invalidate })

  const anySelected = selected.card || selected.directDebit

  const activate = async () => {
    setError(undefined)
    try {
      await activateMut.mutateAsync({ id: connector.id })
    } catch (e) {
      setError(userFacingError(e, `Couldn't activate ${connector.alias}.`, definition.name))
      return
    }
    if (entityId && anySelected && providersQuery.isSuccess) {
      try {
        await routingMut.mutateAsync({
          id: entityId,
          // Merge with the entity's current routing so untouched rails are preserved.
          cardProviderId: selected.card ? connector.id : currentCard?.id,
          directDebitProviderId: selected.directDebit ? connector.id : currentDebit?.id,
          bankAccountId: currentBankId,
        })
      } catch {
        toast.error('Activated, but routing could not be updated. You can set it in payment settings.')
        onDone()
        return
      }
    }
    toast.success(`${connector.alias} is active`)
    onDone()
  }

  const pending = activateMut.isPending || routingMut.isPending

  return (
    <div className="space-y-5">
      <div className="rounded-lg border border-border p-4">
        <AccountDetails
          connector={connector}
          definition={definition}
          extra={webhookNote ? [['Webhook', webhookNote]] : undefined}
        />
      </div>

      {entityId && providersQuery.isSuccess && rails.length > 0 && (
        <div className="space-y-2">
          <p className="text-sm font-medium">
            Use for {defaultEntity?.legalName || 'your default invoicing entity'}
            {entities.length > 1 && (
              <span className="font-normal text-muted-foreground"> (default invoicing entity)</span>
            )}
          </p>
          <div className="overflow-hidden rounded-lg border border-border">
            {rails.map(({ rail, status }, idx) => {
              const current = rail === 'card' ? currentCard : currentDebit
              return (
                <label
                  key={rail}
                  className={cn(
                    'flex items-start gap-3 px-4 py-3',
                    status.enabled ? 'cursor-pointer' : 'cursor-not-allowed opacity-70',
                    idx > 0 && 'border-t border-border'
                  )}
                >
                  <Checkbox
                    checked={status.enabled && selected[rail]}
                    disabled={!status.enabled}
                    onCheckedChange={value =>
                      setSelected(prev => ({ ...prev, [rail]: value === true }))
                    }
                    className="mt-0.5"
                  />
                  <div className="flex-1">
                    <div className="text-sm">{RAIL_LABELS[rail]}</div>
                    <p className="mt-0.5 text-xs text-muted-foreground">
                      {!status.enabled
                        ? `${status.reason ?? ''} You can route it once it's enabled.`
                        : current && current.id !== connector.id
                          ? `Currently routed to ${current.alias}; this replaces it.`
                          : 'Nothing is routed here yet.'}
                    </p>
                  </div>
                </label>
              )
            })}
          </div>
        </div>
      )}

      {entityId && providersQuery.isError && (
        <p className="text-xs text-muted-foreground">
          Couldn&apos;t load your current payment routing, so it can&apos;t be set here. Set it in
          payment routing settings after activating.
        </p>
      )}

      {entities.length > 1 && (
        <p className="text-xs text-muted-foreground">
          Other invoicing entities: Settings → Payment methods.
        </p>
      )}

      {error && <Alert variant="destructive">{error}</Alert>}

      <WizardFooter>
        <Button variant="secondary" size="sm" type="button" onClick={onBack} disabled={pending}>
          Back
        </Button>
        <Button type="button" size="sm" onClick={activate} hasIcon={pending} disabled={pending}>
          {pending && <Spinner />}
          Activate connection
        </Button>
      </WizardFooter>
    </div>
  )
}
