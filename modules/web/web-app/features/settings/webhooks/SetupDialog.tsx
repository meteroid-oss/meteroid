import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
} from '@md/ui'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { useWebhooks } from '@/features/settings/webhooks/useWebhooks'
import { type WebhookEndpoint } from '@/rpc/api/webhooksout/v1/models_pb'

interface Props {
  endpoint: WebhookEndpoint | null
  onClose: () => void
  onEditAll: (endpoint: WebhookEndpoint) => void
}

/**
 * Asks only for what the webhook move could not copy: the values of secret headers.
 * Saving enables the endpoint.
 */
export const SetupDialog = ({ endpoint, onClose, onEditAll }: Props) => {
  const { update } = useWebhooks()
  const [values, setValues] = useState<Record<string, string>>({})
  useEffect(() => {
    setValues({})
  }, [endpoint?.id])

  if (!endpoint) return null

  const missing = endpoint.headers.filter(h => h.sensitive && !h.set)
  const otherHeaders = endpoint.headers.length - missing.length
  const complete = missing.every(h => (values[h.name] ?? '').trim() !== '')

  const save = () =>
    update.mutate(
      {
        id: endpoint.id,
        disabled: false,
        headers: {
          headers: endpoint.headers.map(h => ({
            name: h.name,
            sensitive: h.sensitive,
            value: h.sensitive ? (h.set ? undefined : values[h.name]) : h.value,
          })),
        },
      },
      {
        onSuccess: () => {
          toast.success('Endpoint enabled')
          onClose()
        },
        onError: e => toast.error(e.message),
      }
    )

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>Set up endpoint</DialogTitle>
          <DialogDescription className="truncate font-mono text-xs">
            {endpoint.url}
          </DialogDescription>
        </DialogHeader>
        <p className="text-sm text-muted-foreground">
          This endpoint sends a secret header. We couldn&apos;t copy its value when your webhooks
          were moved, so enter it again.
        </p>
        <div className="space-y-3">
          {missing.map(header => (
            <div key={header.name} className="space-y-1.5">
              <Label htmlFor={`setup-${header.name}`} className="font-mono text-xs">
                {header.name}
              </Label>
              <Input
                id={`setup-${header.name}`}
                type="password"
                autoComplete="off"
                className="font-mono text-xs"
                placeholder="Enter value"
                value={values[header.name] ?? ''}
                onChange={e => setValues(v => ({ ...v, [header.name]: e.target.value }))}
              />
            </div>
          ))}
        </div>
        <p className="text-xs text-muted-foreground">
          Already copied: the URL,{' '}
          {endpoint.eventTypes.length === 0
            ? 'all events'
            : `${endpoint.eventTypes.length} event type${endpoint.eventTypes.length === 1 ? '' : 's'}`}
          {otherHeaders > 0 && `, ${otherHeaders} other header${otherHeaders === 1 ? '' : 's'}`}.{' '}
          <button
            type="button"
            className="underline underline-offset-2 hover:text-foreground"
            onClick={() => onEditAll(endpoint)}
          >
            Edit all settings
          </button>
        </p>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button disabled={!complete || update.isPending} onClick={save}>
            Save and enable
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
