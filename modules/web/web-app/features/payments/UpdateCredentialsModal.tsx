import { useMutation } from '@connectrpc/connect-query'
import { Alert, Button, DialogDescription, DialogTitle, Form, Modal, Spinner } from '@md/ui'
import { useEffect, useMemo, useState } from 'react'
import { Control, useWatch } from 'react-hook-form'
import { useParams } from 'react-router-dom'
import { toast } from 'sonner'

import { useDismissRouteModal } from '@/hooks/useDismissRouteModal'
import { useTenant } from '@/hooks/useTenant'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import {
  getConnectorUsage,
  listConnectors,
  updatePaymentProviderCredentials,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { Connector, ProviderAccount } from '@/rpc/api/connectors/v1/models_pb'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'

import { useInvalidateConnectors } from './ConnectorLifecycle'
import { CredentialFields, CredentialValues } from './CredentialFields'
import { RAIL_LABELS, joinNames, railStatus, userFacingError } from './connectors'
import { ConnectContext, PaymentProviderDefinition, credentialsSchema, paymentProvider } from './providers'

/**
 * Route modal at `manage-connector/:connectorId/credentials`: rotate or fix a connection's keys.
 * Secrets never come back from the server, so every field starts blank and blank means "keep".
 */
export const UpdateCredentialsModal = () => {
  const { connectorId } = useParams<{ connectorId: string }>()
  const closeModal = useDismissRouteModal()
  const connectorsQuery = useQuery(listConnectors, {})
  const connector = connectorsQuery.data?.connectors.find(c => c.id === connectorId)
  const definition = paymentProvider(connector?.provider)

  useEffect(() => {
    if (connectorsQuery.isSuccess && (!connector || !definition)) closeModal()
  }, [connectorsQuery.isSuccess, connector, definition, closeModal])

  if (!connector || !definition) return null
  return <UpdateCredentials connector={connector} definition={definition} onClose={closeModal} />
}

const UpdateCredentials = ({
  connector,
  definition,
  onClose,
}: {
  connector: Connector
  definition: PaymentProviderDefinition
  onClose: () => void
}) => {
  const { connect, name, Logo } = definition
  const { tenant } = useTenant()
  const invalidate = useInvalidateConnectors()
  const ctx: ConnectContext = {
    isProduction: tenant?.environment === TenantEnvironmentEnum.PRODUCTION,
    webhookUrl: connector.webhookUrl,
  }

  const fields = useMemo(
    () => [...connect.credentialFields, ...(connect.webhook ? [connect.webhook.secretField] : [])],
    [connect]
  )
  const schema = useMemo(() => credentialsSchema(connect, fields, 'update'), [connect, fields])
  const methods = useZodForm({
    mode: 'onChange',
    schema,
    defaultValues: Object.fromEntries(fields.map(f => [f, ''])),
  })
  const control = methods.control as unknown as Control<CredentialValues>
  const values = useWatch({ control })
  const anyValue = Object.values(values).some(v => !!v)

  const [error, setError] = useState<string>()
  // Shown instead of closing when the save needs a second look: the keys point at another
  // account or mode, or routed rails are no longer enabled.
  const [result, setResult] = useState<{ accountChange?: AccountChange; lostRouting: string[] }>()

  const usageQuery = useQuery(getConnectorUsage, { id: connector.id })
  const updateMut = useMutation(updatePaymentProviderCredentials, { onSuccess: invalidate })

  const onSubmit = async (submitted: CredentialValues) => {
    setError(undefined)
    try {
      const res = await updateMut.mutateAsync({
        id: connector.id,
        credentials: connect.toCredentials(submitted, ctx),
      })
      const updated = res.connector
      toast.success(
        `Credentials updated${updated?.account ? ` for ${accountLabel(updated.account)}` : ''}`
      )
      const accountChange = accountChangeOf(connector.account, updated?.account)
      const usage = usageQuery.data?.usage
      let lost: string[] = []
      if (updated && usage) {
        lost = [
          ...(usage.routedCard.length && !railStatus(updated, 'card').enabled
            ? [`${RAIL_LABELS.card} for ${joinNames(usage.routedCard.map(e => e.name))}`]
            : []),
          ...(usage.routedDirectDebit.length && !railStatus(updated, 'directDebit').enabled
            ? [
                `${RAIL_LABELS.directDebit} for ${joinNames(usage.routedDirectDebit.map(e => e.name))}`,
              ]
            : []),
        ]
      }
      if (accountChange || lost.length) {
        setResult({ accountChange, lostRouting: lost })
        return
      }
      onClose()
    } catch (e) {
      // Rejected key, unreachable provider: shown inline next to the fields.
      setError(userFacingError(e, connect.errorFallback, name))
    }
  }
  const submit = methods.handleSubmit(onSubmit)

  const account = connector.account

  return (
    <Modal
      visible
      size="medium"
      onCancel={onClose}
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <Logo className="h-6 w-6 text-foreground" />
            <span>Update credentials</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            {connector.alias}
            {account ? ` · ${accountLabel(account)}` : ''}
          </DialogDescription>
        </>
      }
      customFooter={
        result ? (
          <div className="flex w-full justify-end">
            <Button size="sm" onClick={onClose}>
              Done
            </Button>
          </div>
        ) : (
          <div className="flex w-full justify-end space-x-2">
            <Button variant="secondary" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button
              size="sm"
              onClick={submit}
              hasIcon={updateMut.isPending}
              disabled={!anyValue || updateMut.isPending}
            >
              {updateMut.isPending && <Spinner />}
              Verify &amp; save
            </Button>
          </div>
        )
      }
    >
      <Modal.Content>
        {result ? (
          <div className="space-y-3 py-4">
            {result.accountChange && (
              <Alert variant="warning">
                These keys belong to a different {name}{' '}
                {result.accountChange.modeChanged ? 'mode' : 'account'}: now connected to{' '}
                <strong>{result.accountChange.to}</strong>, previously{' '}
                <strong>{result.accountChange.from}</strong>. Customers&apos; saved payment
                methods only work on the account they were created on. If this wasn&apos;t
                intended, update the credentials again with keys from the previous account.
              </Alert>
            )}
            {result.lostRouting.length > 0 && (
              <Alert variant="warning">
                Saved, but {name} no longer reports these routed methods as enabled:{' '}
                {result.lostRouting.join('; ')}. They won&apos;t be collected until they&apos;re
                enabled on the account again, or routed elsewhere in Settings → Payment methods.
              </Alert>
            )}
          </div>
        ) : (
          <Form {...methods}>
            <form
              autoComplete="off"
              className="space-y-5 py-4"
              onSubmit={e => {
                e.preventDefault()
                if (anyValue) submit()
              }}
            >
              <CredentialFields
                connect={connect}
                fields={fields}
                control={control}
                ctx={ctx}
                mode="update"
                keyHint={account?.keyHint}
                onSubmit={() => anyValue && submit()}
              />
              {account?.accountId && (
                <p className="text-xs text-muted-foreground">
                  Use keys from the same {name} account. Saved payment methods only work there.
                </p>
              )}
              {error && <Alert variant="destructive">{error}</Alert>}
            </form>
          </Form>
        )}
      </Modal.Content>
    </Modal>
  )
}

interface AccountChange {
  from: string
  to: string
  modeChanged: boolean
}

const accountLabel = (account: ProviderAccount) =>
  [
    account.displayName,
    account.accountId && `(${account.accountId})`,
    account.livemode ? 'live' : 'test',
  ]
    .filter(Boolean)
    .join(' ')

/** Set when the new keys point at another account or mode than the stored ones. */
const accountChangeOf = (
  before: ProviderAccount | undefined,
  after: ProviderAccount | undefined
): AccountChange | undefined => {
  if (!before || !after) return undefined
  const modeChanged = before.livemode !== after.livemode
  const accountChanged =
    !!before.accountId && !!after.accountId && before.accountId !== after.accountId
  if (!modeChanged && !accountChanged) return undefined
  return { from: accountLabel(before), to: accountLabel(after), modeChanged }
}
