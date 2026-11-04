import { Code } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  Skeleton,
} from '@md/ui'
import { Download, Paperclip, RefreshCw } from 'lucide-react'
import { toast } from 'sonner'

import { EmailHtmlFrame } from '@/components/EmailHtmlFrame'
import { Gate } from '@/components/Gate'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import {
  getSentEmail,
  resendEmail,
} from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import { parseAndFormatDateTime } from '@/utils/date'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import { emailKindLabel } from './EmailKindsView'
import { localeLabel } from './EmailPreviewPanel'

interface EmailPreviewDialogProps {
  // Sent emails share their id with the `entity.email_sent` activity row.
  sentEmailId: string
  open: boolean
  onOpenChange: (open: boolean) => void
}

export const EmailPreviewDialog = ({ sentEmailId, open, onOpenChange }: EmailPreviewDialogProps) => {
  const showConfirmModal = useConfirmationModal()
  const query = useQuery(
    getSentEmail,
    { id: sentEmailId },
    {
      enabled: open,
      staleTime: Infinity,
      // A purged/missing preview is a deterministic NotFound — don't retry it.
      retry: (count, err) =>
        err.code !== Code.NotFound && err.code !== Code.Unimplemented && count < 2,
    }
  )
  const resendMut = useMutation(resendEmail, {
    onSuccess: () => toast.success('Queued'),
    onError: err => toast.error(err.rawMessage || 'Could not resend this email'),
  })

  const email = query.data?.email
  const isGone = query.error?.code === Code.NotFound
  const isUnavailable = query.error?.code === Code.Unimplemented

  const resend = () =>
    showConfirmModal(
      () => resendMut.mutate({ sentEmailId, requestId: crypto.randomUUID() }),
      {
        header: 'Resend this email?',
        message: `It will be rendered again from the current data and sent to ${email?.recipients.join(', ') ?? 'the same recipients'}.`,
      },
      { buttonLabel: 'Resend', danger: false }
    )

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>{email?.subject ?? 'Email preview'}</DialogTitle>
        </DialogHeader>
        {query.isLoading && (
          <div className="space-y-2 py-4">
            <Skeleton height={16} width={240} />
            <Skeleton height={200} width="100%" />
          </div>
        )}
        {query.isError && (
          <div className="py-6 text-sm text-muted-foreground">
            {isGone
              ? 'This email preview is no longer available. Previews are retained for a limited time — the delivery record remains in the activity log.'
              : isUnavailable
                ? 'Email details are not available on this server yet.'
                : 'Couldn’t load this email preview. Please try again.'}
          </div>
        )}
        {email && (
          <div className="space-y-3">
            <div className="text-xs text-muted-foreground space-y-0.5">
              <div>
                <span className="font-medium">From:</span> {email.fromAddr}
              </div>
              <div>
                <span className="font-medium">To:</span> {email.recipients.join(', ')}
              </div>
              {email.replyTo && (
                <div>
                  <span className="font-medium">Reply-To:</span> {email.replyTo}
                </div>
              )}
            </div>
            {email.attachments.length > 0 && (
              <div className="flex flex-wrap gap-2">
                {email.attachments.map((att, i) => {
                  const downloadable = att.id && email.attachmentsShareKey
                  const url = downloadable
                    ? `${env.meteroidRestApiUri}/files/v1/email/attachment/${email.activityId}/${att.id}?token=${email.attachmentsShareKey}`
                    : undefined
                  const className =
                    'inline-flex items-center gap-1.5 rounded border px-2 py-1 text-xs text-muted-foreground'
                  return url ? (
                    <a
                      key={i}
                      href={url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className={`${className} hover:bg-accent hover:text-accent-foreground`}
                    >
                      <Download className="h-3.5 w-3.5" />
                      {att.filename}
                    </a>
                  ) : (
                    <span key={i} className={className}>
                      <Paperclip className="h-3.5 w-3.5" />
                      {att.filename}
                    </span>
                  )
                })}
              </div>
            )}
            <EmailHtmlFrame html={email.bodyHtml} />
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <Badge variant="outline" size="sm">
                {emailKindLabel(email.kind)}
              </Badge>
              {email.locale && (
                <Badge variant="outline" size="sm">
                  {localeLabel(email.locale)}
                </Badge>
              )}
              <span>Sent {parseAndFormatDateTime(email.sentAt)}</span>
              {email.resentFrom && (
                <span className="font-mono" title={email.resentFrom}>
                  · resent from {email.resentFrom.slice(0, 8)}…
                </span>
              )}
              <Gate perm="billing:invoices:manage">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  hasIcon
                  className="ml-auto"
                  disabled={resendMut.isPending}
                  onClick={resend}
                >
                  <RefreshCw size={12} />
                  Resend
                </Button>
              </Gate>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
