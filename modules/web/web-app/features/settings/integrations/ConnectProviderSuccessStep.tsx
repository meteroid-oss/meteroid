import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Button, Checkbox, Spinner, cn } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ArrowRight, CheckCircle2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'

import { useInvoicingEntity } from '@/features/settings/hooks/useInvoicingEntity'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import {
  getInvoicingEntityProviders,
  updateInvoicingEntityProviders,
} from '@/rpc/api/invoicingentities/v1/invoicingentities-InvoicingEntitiesService_connectquery'

interface Props {
  /** The connector that was just created by the wizard. */
  connectorId: string
  /** Alias shown in the confirmation heading (e.g. 'stripe-eu'). */
  connectorAlias: string
  /** Rails this freshly-connected provider is able to serve. */
  capabilities: { card: boolean; directDebit: boolean }
  /** Called when the user is done — applies the chosen routing (if any) then closes. */
  onFinish: () => void
}

/**
 * Final step shown after a payment provider connects. Lets the user route this
 * connection to the card and/or direct-debit rail in one click (applied to the
 * default invoicing entity), mirroring the settings → Payment methods page, and
 * links there for anything more granular (e.g. multiple invoicing entities).
 *
 * Routing is merged with the entity's existing providers so an unchecked rail is
 * never wiped — the backend patch overwrites all three provider slots at once.
 */
export const ConnectProviderSuccessStep = ({
  connectorId,
  connectorAlias,
  capabilities,
  onFinish,
}: Props) => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  const queryClient = useQueryClient()
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

  const [setAsCard, setSetAsCard] = useState(false)
  const [setAsDebit, setSetAsDebit] = useState(false)
  const [initialized, setInitialized] = useState(false)

  // Default each supported rail ON when nothing is routed there yet, so the common
  // "first provider" case is a single confirmation. Leave it OFF (opt-in) when the
  // rail already points somewhere, to avoid silently repointing existing routing.
  useEffect(() => {
    if (initialized || !entityId || !providersQuery.isSuccess) return
    setSetAsCard(capabilities.card && !currentCard)
    setSetAsDebit(capabilities.directDebit && !currentDebit)
    setInitialized(true)
  }, [initialized, entityId, providersQuery.isSuccess, capabilities, currentCard, currentDebit])

  const updateMut = useMutation(updateInvoicingEntityProviders, {
    onSuccess: () => {
      // Remove (not just invalidate) the cached providers for every entity.
      // The settings → Payment methods form seeds its react-hook-form defaults
      // from this query on mount and is keyed only by entity id, so it never
      // picks up a background refetch — invalidation would leave it showing the
      // pre-connect routing until a hard refresh. Dropping the cache forces the
      // page to reload fresh data.
      queryClient.removeQueries({
        queryKey: createConnectQueryKey({
          schema: getInvoicingEntityProviders,
          cardinality: undefined,
        }),
      })
    },
  })

  const anySelected = setAsCard || setAsDebit

  const finish = async () => {
    if (entityId && anySelected && providersQuery.isSuccess) {
      try {
        await updateMut.mutateAsync({
          id: entityId,
          // Merge with the entity's current routing so untouched rails are preserved.
          cardProviderId: setAsCard ? connectorId : currentCard?.id,
          directDebitProviderId: setAsDebit ? connectorId : currentDebit?.id,
          bankAccountId: currentBankId,
        })
        toast.success('Payment routing updated')
      } catch {
        toast.error(
          'Connected, but routing could not be updated. You can set it in payment settings.'
        )
      }
    }
    onFinish()
  }

  const rails = [
    capabilities.card && {
      key: 'card',
      icon: '💳',
      label: 'Credit card',
      checked: setAsCard,
      onChange: setSetAsCard,
      current: currentCard,
    },
    capabilities.directDebit && {
      key: 'directDebit',
      icon: '⬇️',
      label: 'Direct debit (SEPA, BACS, ACH)',
      checked: setAsDebit,
      onChange: setSetAsDebit,
      current: currentDebit,
    },
  ].filter((r): r is Exclude<typeof r, false> => Boolean(r))

  return (
    <div className="space-y-6">
      <div className="flex flex-col items-center text-center gap-2">
        <CheckCircle2 className="w-12 h-12 text-success" strokeWidth={1.2} />
        <h3 className="text-md font-semibold">{connectorAlias} connected</h3>
        <p className="text-muted-foreground text-sm">
          Route payments through this connection now, or set it up later in payment settings.
        </p>
      </div>

      {entityId && providersQuery.isSuccess && (
        <div className="overflow-hidden rounded-lg border border-border">
          {rails.map((rail, idx) => (
            <label
              key={rail.key}
              className={cn(
                'flex cursor-pointer items-start gap-3 px-4 py-3',
                idx > 0 && 'border-t border-border'
              )}
            >
              <Checkbox
                checked={rail.checked}
                onCheckedChange={value => rail.onChange(value === true)}
                className="mt-0.5"
              />
              <div className="flex-1">
                <div className="flex items-center gap-2 text-sm">
                  <span>{rail.icon}</span>
                  <span>Use for {rail.label}</span>
                </div>
                {rail.current && rail.current.id !== connectorId && (
                  <p className="mt-0.5 text-xs text-muted-foreground">
                    Currently routed to {rail.current.alias} — this replaces it.
                  </p>
                )}
              </div>
            </label>
          ))}
        </div>
      )}

      {entityId && providersQuery.isError && (
        <p className="text-xs text-muted-foreground">
          Couldn’t load your current payment routing, so it can’t be set here. Your provider is
          connected — set the default rail in payment routing settings.
        </p>
      )}

      {entities.length > 1 && (
        <p className="text-xs text-muted-foreground">
          Applies to your default invoicing entity
          {defaultEntity?.legalName ? ` (${defaultEntity.legalName})` : ''}. Configure other
          entities in payment routing.
        </p>
      )}

      <div className="text-center">
        <Button
          type="button"
          variant="link"
          size="sm"
          hasIcon
          className="h-fit px-0"
          onClick={() => navigate(`${basePath}/settings/payments`)}
        >
          Open payment routing settings
          <ArrowRight size={14} strokeWidth={1.5} />
        </Button>
      </div>

      <div className="flex justify-end gap-2 border-t px-5 py-3">
        <Button variant="secondary" size="sm" type="button" onClick={onFinish}>
          Skip
        </Button>
        <Button
          type="button"
          size="sm"
          onClick={finish}
          hasIcon={updateMut.isPending}
          disabled={updateMut.isPending}
        >
          {updateMut.isPending && <Spinner />}
          {anySelected ? 'Apply & finish' : 'Finish'}
        </Button>
      </div>
    </div>
  )
}
