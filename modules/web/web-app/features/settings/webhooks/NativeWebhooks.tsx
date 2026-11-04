import { Button } from '@md/ui'
import { ArrowLeftIcon } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { useLocation, useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { EndpointDialog } from '@/features/settings/webhooks/EndpointDialog'
import { EndpointPage, type EndpointTab } from '@/features/settings/webhooks/EndpointPage'
import { EndpointsList } from '@/features/settings/webhooks/EndpointsList'
import { EventCatalogPage } from '@/features/settings/webhooks/EventCatalogPage'
import { SecretDialog, type SecretDialogState } from '@/features/settings/webhooks/SecretDialog'
import { SetupDialog } from '@/features/settings/webhooks/SetupDialog'
import { useWebhooks } from '@/features/settings/webhooks/useWebhooks'
import { type WebhookEndpoint } from '@/rpc/api/webhooksout/v1/models_pb'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

type View =
  { kind: 'list' } | { kind: 'endpoint'; id: string; tab?: EndpointTab } | { kind: 'catalog' }

/** settings/webhooks, settings/webhooks/endpoints/<id>[?tab=…], settings/webhooks/catalog */
const useView = (): View => {
  const { pathname, search } = useLocation()
  const endpoint = pathname.match(/\/webhooks\/endpoints\/([^/]+)\/?$/)
  if (endpoint) {
    const tab = new URLSearchParams(search).get('tab')
    return {
      kind: 'endpoint',
      id: endpoint[1],
      tab: tab === 'testing' || tab === 'advanced' ? tab : undefined,
    }
  }
  if (/\/webhooks\/catalog\/?$/.test(pathname)) return { kind: 'catalog' }
  return { kind: 'list' }
}

export const NativeWebhooks = () => {
  const view = useView()
  const navigate = useNavigate()
  const { endpoints, isLoading, update, archive, rotateSecret } = useWebhooks()
  const confirm = useConfirmationModal()

  const [dialogOpen, setDialogOpen] = useState(false)
  const [editing, setEditing] = useState<WebhookEndpoint | undefined>(undefined)
  const [secretDialog, setSecretDialog] = useState<SecretDialogState | null>(null)
  const [settingUp, setSettingUp] = useState<WebhookEndpoint | null>(null)

  // Relative to the settings tab route, so these stay inside the settings modal.
  const toList = () => navigate('.')
  const toEndpoint = (id: string, tab?: EndpointTab) =>
    navigate(tab ? `endpoints/${id}?tab=${tab}` : `endpoints/${id}`)
  const toCatalog = () => navigate('catalog')

  const openEditor = (endpoint?: WebhookEndpoint) => {
    setEditing(endpoint)
    setDialogOpen(true)
  }

  const toggleDisabled = (endpoint: WebhookEndpoint) =>
    update.mutate(
      { id: endpoint.id, disabled: !endpoint.disabled },
      {
        onSuccess: () =>
          toast.success(endpoint.disabled ? 'Endpoint enabled' : 'Endpoint disabled'),
        onError: e => toast.error(e.message),
      }
    )

  const confirmRotate = (endpoint: WebhookEndpoint) =>
    confirm(
      () =>
        rotateSecret.mutate(
          { id: endpoint.id },
          {
            onSuccess: res =>
              setSecretDialog({
                endpointId: endpoint.id,
                secret: res.secret,
                title: 'New signing secret',
                description:
                  'Copy it now. The previous secret keeps signing for 24 hours so you can roll over without dropping events.',
              }),
            onError: e => toast.error(e.message),
          }
        ),
      {
        header: 'Rotate signing secret',
        message: `Rotate the secret for ${endpoint.url}? The current secret stays valid for 24 hours, then stops signing.`,
      },
      { buttonLabel: 'Rotate secret' }
    )

  const confirmRemove = (endpoint: WebhookEndpoint) =>
    confirm(
      () =>
        archive.mutate(
          { id: endpoint.id },
          {
            onSuccess: () => {
              toast.success('Endpoint removed')
              if (view.kind === 'endpoint') toList()
            },
            onError: e => toast.error(e.message),
          }
        ),
      {
        header: 'Remove endpoint',
        message: `Remove ${endpoint.url}? Pending deliveries are dropped and it stops receiving events.`,
      },
      { buttonLabel: 'Remove endpoint' }
    )

  const revealSecret = (endpoint: WebhookEndpoint) =>
    setSecretDialog({
      endpointId: endpoint.id,
      title: 'Signing secret',
      description:
        'Verify the webhook-signature header with this secret. Never expose it client-side.',
    })

  let content: ReactNode
  if (view.kind === 'catalog') {
    content = <EventCatalogPage onBack={toList} />
  } else if (view.kind === 'endpoint') {
    const endpoint = endpoints.find(e => e.id === view.id)
    content = endpoint ? (
      <EndpointPage
        key={`${endpoint.id}-${view.tab ?? 'overview'}`}
        endpoint={endpoint}
        initialTab={view.tab}
        onBack={toList}
        onEdit={() => openEditor(endpoint)}
        onSetUp={() => setSettingUp(endpoint)}
        onRevealSecret={() => revealSecret(endpoint)}
        onRotateSecret={() => confirmRotate(endpoint)}
        onToggleDisabled={() => toggleDisabled(endpoint)}
        onRemove={() => confirmRemove(endpoint)}
      />
    ) : isLoading ? null : (
      <div className="space-y-3">
        <Button variant="ghost" size="sm" hasIcon className="-ml-2" onClick={toList}>
          <ArrowLeftIcon size={14} /> Webhooks
        </Button>
        <p className="text-sm text-muted-foreground">This endpoint no longer exists.</p>
      </div>
    )
  } else {
    content = (
      <EndpointsList
        endpoints={endpoints}
        isLoading={isLoading}
        onOpen={endpoint => toEndpoint(endpoint.id)}
        onAdd={() => openEditor()}
        onEdit={openEditor}
        onSetUp={setSettingUp}
        onToggleDisabled={toggleDisabled}
        onRemove={confirmRemove}
        onOpenCatalog={toCatalog}
      />
    )
  }

  return (
    <div className="flex max-w-4xl flex-col gap-6">
      {content}

      <EndpointDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        editing={editing}
        onCreated={(endpointId, secret) =>
          setSecretDialog({
            endpointId,
            secret,
            title: 'Endpoint added',
            description:
              "Copy the signing secret now. You can reveal it again from the endpoint's Advanced tab.",
            action: {
              label: 'Send a test event',
              onClick: () => toEndpoint(endpointId, 'testing'),
            },
            onDismiss: () => toEndpoint(endpointId),
          })
        }
      />

      <SecretDialog state={secretDialog} onClose={() => setSecretDialog(null)} />

      {settingUp && (
        <SetupDialog
          endpoint={settingUp}
          onClose={() => setSettingUp(null)}
          onEditAll={endpoint => {
            setSettingUp(null)
            openEditor(endpoint)
          }}
        />
      )}
    </div>
  )
}
