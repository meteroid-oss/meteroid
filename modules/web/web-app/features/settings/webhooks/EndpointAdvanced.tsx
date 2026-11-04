import { Button, Input } from '@md/ui'
import { useState, type ReactNode } from 'react'
import { toast } from 'sonner'

import { useWebhooks } from '@/features/settings/webhooks/useWebhooks'
import { type WebhookEndpoint } from '@/rpc/api/webhooksout/v1/models_pb'

interface Props {
  endpoint: WebhookEndpoint
  onRevealSecret: () => void
  onRotateSecret: () => void
  onEditHeaders: () => void
  onToggleDisabled: () => void
  onRemove: () => void
}

/** One setting per row: what it is on the left, its control on the right. */
const Row = ({
  title,
  description,
  children,
}: {
  title: string
  description?: ReactNode
  children?: ReactNode
}) => (
  <div className="flex flex-col gap-3 py-4 sm:flex-row sm:items-start sm:justify-between">
    <div className="min-w-0 space-y-0.5">
      <div className="text-sm font-medium">{title}</div>
      {description && <div className="text-sm text-muted-foreground">{description}</div>}
    </div>
    <div className="flex shrink-0 items-center gap-2">{children}</div>
  </div>
)

export const EndpointAdvanced = ({
  endpoint,
  onRevealSecret,
  onRotateSecret,
  onEditHeaders,
  onToggleDisabled,
  onRemove,
}: Props) => {
  const { update } = useWebhooks()
  const [editingLimit, setEditingLimit] = useState(false)
  const [limit, setLimit] = useState('')

  const parsedLimit = limit.trim() === '' ? undefined : Number(limit)
  const limitValid =
    parsedLimit === undefined ||
    (Number.isInteger(parsedLimit) && parsedLimit >= 1 && parsedLimit <= 1000)

  const saveLimit = (perSec: number | undefined) =>
    update.mutate(
      { id: endpoint.id, rateLimit: { perSec } },
      {
        onSuccess: () => {
          toast.success(perSec ? 'Rate limit saved' : 'Rate limit removed')
          setEditingLimit(false)
        },
        onError: e => toast.error(e.message),
      }
    )

  return (
    <div className="divide-y divide-border">
      <Row
        title="Signing secret"
        description="Verify the webhook-signature header with it. Keep it server-side."
      >
        <span className="font-mono text-xs text-muted-foreground">whsec_••••••••••••</span>
        <Button variant="outline" size="sm" onClick={onRevealSecret}>
          Reveal
        </Button>
        <Button variant="ghost" size="sm" onClick={onRotateSecret}>
          Rotate…
        </Button>
      </Row>

      <Row
        title="Custom headers"
        description={
          endpoint.headers.length === 0 ? (
            'Sent with every delivery. None yet.'
          ) : (
            <span className="space-y-0.5">
              <span className="block">Sent with every delivery.</span>
              {endpoint.headers.map(header => (
                <span key={header.name} className="block font-mono text-xs">
                  {header.name}:{' '}
                  {header.sensitive
                    ? header.set
                      ? '•••••••• secret'
                      : 'value missing'
                    : header.value}
                </span>
              ))}
            </span>
          )
        }
      >
        <Button variant="outline" size="sm" onClick={onEditHeaders}>
          Edit headers
        </Button>
      </Row>

      <Row
        title="Rate limit"
        description="Cap how many events per second we send to this endpoint."
      >
        {editingLimit ? (
          <>
            <Input
              className="h-8 w-24"
              inputMode="numeric"
              autoFocus
              value={limit}
              onChange={e => setLimit(e.target.value)}
              placeholder="per sec"
            />
            <Button
              size="sm"
              disabled={!limitValid || parsedLimit === undefined || update.isPending}
              onClick={() => saveLimit(parsedLimit)}
            >
              Save
            </Button>
            <Button variant="ghost" size="sm" onClick={() => setEditingLimit(false)}>
              Cancel
            </Button>
          </>
        ) : (
          <>
            <span className="text-sm text-muted-foreground">
              {endpoint.rateLimitPerSec ? `${endpoint.rateLimitPerSec} per second` : 'No limit'}
            </span>
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setLimit(endpoint.rateLimitPerSec ? String(endpoint.rateLimitPerSec) : '')
                setEditingLimit(true)
              }}
            >
              {endpoint.rateLimitPerSec ? 'Change' : 'Set limit'}
            </Button>
            {endpoint.rateLimitPerSec && (
              <Button variant="ghost" size="sm" onClick={() => saveLimit(undefined)}>
                Remove
              </Button>
            )}
          </>
        )}
      </Row>
      {editingLimit && !limitValid && (
        <p className="pb-2 text-xs text-destructive">Enter a whole number from 1 to 1000.</p>
      )}

      <Row
        title={endpoint.disabled ? 'Enable endpoint' : 'Disable endpoint'}
        description={
          endpoint.disabled
            ? 'Start sending new events to this endpoint again.'
            : 'Stop sending events to this endpoint until you enable it again. Events raised while it is disabled are not kept.'
        }
      >
        <Button
          variant="outline"
          size="sm"
          disabled={endpoint.needsSetup}
          onClick={onToggleDisabled}
        >
          {endpoint.disabled ? 'Enable' : 'Disable'}
        </Button>
      </Row>

      <Row
        title="Remove endpoint"
        description="Pending deliveries are dropped. This can't be undone."
      >
        <Button variant="destructiveGhost" size="sm" onClick={onRemove}>
          Remove…
        </Button>
      </Row>
    </div>
  )
}
