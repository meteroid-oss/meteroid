import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Button,
  InputWithIcon,
  Skeleton,
} from '@md/ui'
import { CopyIcon } from 'lucide-react'
import { toast } from 'sonner'

import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import { getWebhookEndpointSecret } from '@/rpc/api/webhooksout/v1/webhooksout-WebhooksService_connectquery'

export interface SecretDialogState {
  endpointId: string
  title: string
  description: string
  /** Set when the secret was just returned by create/rotate; otherwise it is fetched on open. */
  secret?: string
  /** Replaces "Done" with this, e.g. "Send a test event" right after create. */
  action?: { label: string; onClick: () => void }
  /** Runs when the dialog is closed without the action. */
  onDismiss?: () => void
}

interface Props {
  state: SecretDialogState | null
  onClose: () => void
}

export const SecretDialog = ({ state, onClose }: Props) => {
  const dismiss = () => {
    state?.onDismiss?.()
    onClose()
  }

  const needsFetch = state != null && state.secret === undefined
  const secretQuery = useQuery(
    getWebhookEndpointSecret,
    { id: state?.endpointId ?? '' },
    { enabled: needsFetch }
  )

  const secret = state?.secret ?? (needsFetch ? secretQuery.data?.secret : undefined)

  return (
    <Dialog open={state != null} onOpenChange={open => !open && dismiss()}>
      <DialogContent className="sm:max-w-[520px]">
        <DialogHeader>
          <DialogTitle>{state?.title}</DialogTitle>
          <DialogDescription>{state?.description}</DialogDescription>
        </DialogHeader>

        {secret ? (
          <InputWithIcon
            value={secret}
            readOnly
            icon={<CopyIcon className="group-hover:text-success" />}
            className="cursor-pointer font-mono text-xs"
            containerClassName="group"
            onClick={() => copyToClipboard(secret, () => toast.success('Secret copied'))}
          />
        ) : secretQuery.isError ? (
          <p className="text-sm text-destructive">Unable to read the signing secret.</p>
        ) : (
          <Skeleton height={36} width="100%" />
        )}

        <DialogFooter>
          {state?.action ? (
            <Button
              onClick={() => {
                state.action?.onClick()
                onClose()
              }}
            >
              {state.action.label}
            </Button>
          ) : (
            <Button onClick={dismiss}>Done</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
