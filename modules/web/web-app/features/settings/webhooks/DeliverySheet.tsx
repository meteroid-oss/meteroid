import { json } from '@codemirror/lang-json'
import {
  Button,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  Skeleton,
} from '@md/ui'
import { githubDark, githubLight } from '@uiw/codemirror-theme-github'
import CodeMirror from '@uiw/react-codemirror'
import { useState, type ReactNode } from 'react'
import { toast } from 'sonner'

import { AttemptOutput } from '@/features/settings/webhooks/AttemptOutput'
import { DeliveryStatusBadge } from '@/features/settings/webhooks/badges'
import { useWebhooks } from '@/features/settings/webhooks/useWebhooks'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import { WebhookDeliveryStatusEnum, type WebhookAttempt } from '@/rpc/api/webhooksout/v1/models_pb'
import { getWebhookDelivery } from '@/rpc/api/webhooksout/v1/webhooksout-WebhooksService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'
import { useTheme } from 'providers/ThemeProvider'

/** Attempts beyond this collapse their middle so a long retry run stays short. */
const COLLAPSE_ABOVE = 4

interface Props {
  deliveryId: string | null
  endpointUrl: string
  onClose: () => void
}

const prettyPayload = (payload: string) => {
  try {
    return JSON.stringify(JSON.parse(payload), null, 2)
  } catch {
    return payload
  }
}

/** One event sent to one endpoint: why it failed first, then what was sent. */
export const DeliverySheet = ({ deliveryId, endpointUrl, onClose }: Props) => {
  const { isDarkMode } = useTheme()
  const { resend } = useWebhooks()
  const [expanded, setExpanded] = useState(false)
  const detailQuery = useQuery(
    getWebhookDelivery,
    { id: deliveryId ?? '' },
    { enabled: deliveryId != null }
  )

  const delivery = detailQuery.data?.delivery
  const payload = prettyPayload(detailQuery.data?.payload ?? '')
  const attempts = [...(detailQuery.data?.attempts ?? [])].sort(
    (a, b) => b.attemptNumber - a.attemptNumber
  )
  const collapsible = attempts.length > COLLAPSE_ABOVE && !expanded
  const shown = collapsible ? [attempts[0], attempts[attempts.length - 1]] : attempts
  const hidden = attempts.length - shown.length

  const retrying =
    delivery?.status === WebhookDeliveryStatusEnum.PENDING && delivery.nextAttemptAt != null

  return (
    <Sheet
      open={deliveryId != null}
      onOpenChange={open => {
        if (!open) {
          setExpanded(false)
          onClose()
        }
      }}
    >
      <SheetContent size="large" className="gap-0 overflow-y-auto">
        <SheetHeader className="pb-4">
          <div className="flex items-center justify-between gap-3 pr-8">
            <SheetTitle className="flex items-center gap-2 font-mono text-sm">
              {delivery?.eventType ?? 'Delivery'}
              {delivery && <DeliveryStatusBadge status={delivery.status} />}
            </SheetTitle>
            <Button
              size="sm"
              disabled={!delivery || resend.isPending}
              onClick={() =>
                deliveryId &&
                resend.mutate(
                  { id: deliveryId },
                  {
                    onSuccess: () => toast.success('Delivery re-queued'),
                    onError: e => toast.error(e.message),
                  }
                )
              }
            >
              Resend
            </Button>
          </div>
          <SheetDescription className="truncate font-mono text-xs">{endpointUrl}</SheetDescription>
        </SheetHeader>

        {detailQuery.isLoading || !delivery ? (
          <div className="space-y-2">
            <Skeleton height={20} width="100%" />
            <Skeleton height={160} width="100%" />
          </div>
        ) : (
          <div className="space-y-6">
            <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-[13px] sm:grid-cols-4">
              <Fact label="Created">{parseAndFormatDateTime(delivery.createdAt, 'sec')}</Fact>
              {retrying ? (
                <Fact label="Next try">
                  {parseAndFormatDateTime(delivery.nextAttemptAt ?? '', 'sec')}
                </Fact>
              ) : (
                <Fact
                  label={
                    delivery.status === WebhookDeliveryStatusEnum.FAILED ? 'Gave up' : 'Completed'
                  }
                >
                  {delivery.completedAt ? parseAndFormatDateTime(delivery.completedAt, 'sec') : '—'}
                </Fact>
              )}
              <Fact label="Attempts">{Number(delivery.attempts)}</Fact>
              <Fact label="Event id">
                <span className="break-all font-mono text-xs">{delivery.messageId}</span>
              </Fact>
            </dl>

            <section className="space-y-2">
              <h4 className="text-xs font-medium text-muted-foreground">Attempts</h4>
              {attempts.length === 0 ? (
                <p className="text-sm text-muted-foreground">No attempt yet.</p>
              ) : (
                <ol className="space-y-2">
                  {shown.map((attempt, index) => (
                    <li key={attempt.id}>
                      <AttemptRow attempt={attempt} />
                      {collapsible && index === 0 && hidden > 0 && (
                        <button
                          type="button"
                          className="mt-2 w-full rounded-md border border-dashed border-border py-1.5 text-xs text-muted-foreground hover:text-foreground"
                          onClick={() => setExpanded(true)}
                        >
                          {hidden} more attempt{hidden === 1 ? '' : 's'} · Show
                        </button>
                      )}
                    </li>
                  ))}
                </ol>
              )}
            </section>

            <section className="space-y-1.5">
              <div className="flex items-center justify-between">
                <h4 className="text-xs font-medium text-muted-foreground">Payload</h4>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => copyToClipboard(payload, () => toast.success('Payload copied'))}
                >
                  Copy
                </Button>
              </div>
              <div className="overflow-hidden rounded-lg border border-border">
                <CodeMirror
                  value={payload}
                  extensions={[json()]}
                  theme={isDarkMode ? githubDark : githubLight}
                  editable={false}
                  readOnly
                  basicSetup={{ foldGutter: false, highlightActiveLine: false }}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                Signed with the webhook-id, webhook-timestamp and webhook-signature headers.
              </p>
            </section>
          </div>
        )}
      </SheetContent>
    </Sheet>
  )
}

const AttemptRow = ({ attempt }: { attempt: WebhookAttempt }) => (
  <div className="rounded-lg border border-border px-3 py-2">
    <div className="flex items-center gap-3 text-xs">
      <span className="font-medium">#{attempt.attemptNumber}</span>
      <span className="text-muted-foreground">
        {parseAndFormatDateTime(attempt.attemptedAt, 'sec')}
      </span>
      <span
        className={`ml-auto font-medium ${attempt.succeeded ? 'text-success' : 'text-destructive'}`}
      >
        {attempt.responseStatus ?? 'No response'}
      </span>
      <span className="w-20 text-right text-muted-foreground">
        {attempt.durationMs.toLocaleString()} ms
      </span>
    </div>
    <AttemptOutput attempt={attempt} />
  </div>
)

const Fact = ({ label, children }: { label: string; children: ReactNode }) => (
  <div>
    <dt className="text-xs text-muted-foreground">{label}</dt>
    <dd className="mt-0.5 min-w-0">{children}</dd>
  </div>
)
