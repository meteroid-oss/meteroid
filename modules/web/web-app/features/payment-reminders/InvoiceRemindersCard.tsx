import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  cn,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Flex,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Separator,
  Skeleton,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import {
  Check,
  CircleDot,
  CircleHelp,
  Minus,
  OctagonX,
  Pause,
  Play,
  Redo2,
  RotateCcw,
  Send,
} from 'lucide-react'
import { ReactNode, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { EmailHtmlFrame } from '@/components/EmailHtmlFrame'
import { Gate } from '@/components/Gate'
import { EmailPreviewDialog } from '@/features/customer-emails/EmailPreviewDialog'
import { TestBannerNote } from '@/features/customer-emails/TestBannerNote'
import { offsetPill } from '@/features/customer-emails/offsetPill'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { parseAndFormatDateTime } from '@/lib/utils/date'
import { previewInvoiceEmail } from '@/rpc/api/customeremails/v1/customeremails-CustomerEmailsService_connectquery'
import { EmailKind } from '@/rpc/api/customeremails/v1/models_pb'
import {
  InvoiceReminderState,
  InvoiceReminderStepStatus,
  InvoiceReminderStepView,
  NotEligibleReason,
  ReminderEnrolledVia,
  ReminderStopReason,
  SequenceRef,
} from '@/rpc/api/paymentreminders/v1/models_pb'
import {
  getInvoiceReminders,
  pauseInvoiceReminders,
  previewInvoiceReminderResolution,
  restartInvoiceReminders,
  resumeInvoiceReminders,
  sendInvoiceReminderNow,
  stopInvoiceReminders,
} from '@/rpc/api/paymentreminders/v1/paymentreminders-PaymentRemindersService_connectquery'

const PAUSE_PRESETS = [7, 14, 30]

/** An enrolled invoice the sweeper holds: nothing is sent until the hold is released (D5). */
const HOLD_REASONS = new Set<NotEligibleReason>([
  NotEligibleReason.KIND_DISABLED,
  NotEligibleReason.EMAILS_DISABLED,
  NotEligibleReason.TENANT_INACTIVE,
  NotEligibleReason.NO_ENABLED_SEQUENCE,
])

/** Reasons the server refuses a Restart for, so the button would only produce an error. */
const RESTART_BLOCKED = new Set<NotEligibleReason>([
  NotEligibleReason.NOT_PAYABLE,
  NotEligibleReason.NO_DUE_DATE,
  NotEligibleReason.NO_PDF,
  NotEligibleReason.CONSOLIDATED_CHILD,
  NotEligibleReason.AUTO_CHARGED,
  NotEligibleReason.CUSTOMER_OPTED_OUT,
])

export const notEligibleLabel = (reason: NotEligibleReason): string => {
  switch (reason) {
    case NotEligibleReason.NOT_PAYABLE:
      return 'not payable'
    case NotEligibleReason.NO_DUE_DATE:
      return 'no due date'
    case NotEligibleReason.NO_PDF:
      return 'PDF not generated yet'
    case NotEligibleReason.CONSOLIDATED_CHILD:
      return 'part of a consolidated invoice'
    case NotEligibleReason.AUTO_CHARGED:
      return 'charged automatically'
    case NotEligibleReason.CUSTOMER_OPTED_OUT:
      return 'customer opted out'
    case NotEligibleReason.STOPPED:
      return 'stopped'
    case NotEligibleReason.TENANT_INACTIVE:
      return 'workspace inactive'
    case NotEligibleReason.EMAILS_DISABLED:
      return 'customer emails switched off'
    case NotEligibleReason.KIND_DISABLED:
      return 'payment reminders switched off'
    case NotEligibleReason.NO_ENABLED_SEQUENCE:
      return 'no matching sequence'
    case NotEligibleReason.LADDER_STALE:
      return 'schedule already elapsed'
    default:
      return 'unknown reason'
  }
}

export const enrolledViaLabel = (via: ReminderEnrolledVia): string | undefined => {
  switch (via) {
    case ReminderEnrolledVia.CUSTOMER_OVERRIDE:
      return 'Customer override'
    case ReminderEnrolledVia.AUDIENCE:
      return 'Audience match'
    case ReminderEnrolledVia.DEFAULT:
      return 'Default sequence'
    default:
      return undefined
  }
}

export const stopReasonLabel = (reason: ReminderStopReason): string => {
  switch (reason) {
    case ReminderStopReason.OPERATOR:
      return 'by operator'
    case ReminderStopReason.CUSTOMER_OPT_OUT:
      return 'customer opted out'
    case ReminderStopReason.SEQUENCE_ARCHIVED:
      return 'sequence archived'
    case ReminderStopReason.PAYMENT_DISPUTED:
      return 'payment disputed'
    default:
      return 'unknown reason'
  }
}

type Phase =
  'not_eligible' | 'waiting' | 'active' | 'held' | 'paused' | 'stopped' | 'completed' | 'ended'

const phaseOf = (state: InvoiceReminderState, now: Date): Phase => {
  if (state.endedAt) return 'ended'
  if (state.stoppedAt) return 'stopped'
  if (state.completedAt) return 'completed'
  if (state.sequence) {
    if (!state.eligible && HOLD_REASONS.has(state.notEligibleReason)) return 'held'
    return state.pausedUntil && new Date(state.pausedUntil) > now ? 'paused' : 'active'
  }
  return state.eligible ? 'waiting' : 'not_eligible'
}

const StatusBadge = ({ state, phase }: { state: InvoiceReminderState; phase: Phase }) => {
  switch (phase) {
    case 'not_eligible':
      return (
        <Badge variant="secondary" size="sm">
          Not eligible: {notEligibleLabel(state.notEligibleReason)}
        </Badge>
      )
    case 'waiting':
      return (
        <Badge variant="outline" size="sm">
          Waiting
        </Badge>
      )
    case 'active':
      return (
        <Badge variant="success" size="sm">
          Active
        </Badge>
      )
    case 'held':
      return (
        <Badge variant="warning" size="sm">
          Held: {notEligibleLabel(state.notEligibleReason)}
        </Badge>
      )
    case 'paused':
      return (
        <Badge variant="warning" size="sm">
          Paused until {parseAndFormatDateTime(state.pausedUntil ?? '')}
        </Badge>
      )
    case 'stopped':
      return (
        <Badge variant="secondary" size="sm">
          Stopped: {stopReasonLabel(state.stopReason)}
        </Badge>
      )
    case 'completed':
      return (
        <Badge variant="outline" size="sm">
          Completed
        </Badge>
      )
    case 'ended':
      return (
        <Badge variant="secondary" size="sm">
          Ended
        </Badge>
      )
  }
}

const Row = ({ title, value }: { title: string; value: ReactNode }) => (
  <Flex align="start" justify="between" className="gap-4">
    <div className="text-[13px] text-muted-foreground shrink-0">{title}</div>
    <div className="text-[13px] text-right min-w-0">{value}</div>
  </Flex>
)

const stepIcon = (status: InvoiceReminderStepStatus): ReactNode => {
  switch (status) {
    case InvoiceReminderStepStatus.SENT:
      return <Check size={12} className="text-success" />
    case InvoiceReminderStepStatus.SKIPPED_CATCH_UP:
      return <Redo2 size={12} className="text-muted-foreground" />
    case InvoiceReminderStepStatus.SCHEDULED:
      return <CircleDot size={12} className="text-foreground" />
    case InvoiceReminderStepStatus.DUE:
      return <CircleDot size={12} className="text-warning" />
    default:
      return <Minus size={12} className="text-muted-foreground" />
  }
}

const stepStatusLabel = (status: InvoiceReminderStepStatus): string => {
  switch (status) {
    case InvoiceReminderStepStatus.SENT:
      return 'Sent'
    case InvoiceReminderStepStatus.SKIPPED_CATCH_UP:
      return 'Skipped (catch-up)'
    case InvoiceReminderStepStatus.SCHEDULED:
      return 'Scheduled'
    case InvoiceReminderStepStatus.DUE:
      return 'Due'
    default:
      return 'Not schedulable'
  }
}

const StepTimeline = ({
  steps,
  muted,
  onViewSent,
  onPreview,
}: {
  steps: InvoiceReminderStepView[]
  muted: boolean
  onViewSent: (sentEmailId: string) => void
  onPreview?: (offsetDays: number) => void
}) => (
  <ul className={cn('flex flex-col gap-1', muted && 'opacity-60')}>
    {steps.map(step => {
      const previewable =
        onPreview &&
        (step.status === InvoiceReminderStepStatus.SCHEDULED ||
          step.status === InvoiceReminderStepStatus.DUE)
      return (
        <li
          key={step.offsetDays}
          className="flex items-center gap-2 text-[13px]"
          title={stepStatusLabel(step.status)}
        >
          <span className="flex h-4 w-4 shrink-0 items-center justify-center">
            {stepIcon(step.status)}
          </span>
          <span className="w-[88px] shrink-0">{offsetPill(step.offsetDays)}</span>
          <span className="text-muted-foreground min-w-0 truncate">
            {step.status === InvoiceReminderStepStatus.NOT_SCHEDULABLE
              ? 'Not schedulable'
              : parseAndFormatDateTime(step.fireAt)}
          </span>
          {step.status === InvoiceReminderStepStatus.SENT && step.sentEmailId && (
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto h-5 px-2 text-[11px]"
              onClick={() => onViewSent(step.sentEmailId ?? '')}
            >
              View
            </Button>
          )}
          {previewable && (
            <Button
              variant="ghost"
              size="sm"
              className="ml-auto h-5 px-2 text-[11px]"
              onClick={() => onPreview(step.offsetDays)}
            >
              Preview
            </Button>
          )}
        </li>
      )
    })}
  </ul>
)

const StepPreviewDialog = ({
  invoiceId,
  offsetDays,
  onClose,
}: {
  invoiceId: string
  offsetDays: number | null
  onClose: () => void
}) => {
  const open = offsetDays !== null
  const query = useQuery(
    previewInvoiceEmail,
    { invoiceId, kind: EmailKind.PAYMENT_REMINDER, offsetDays: offsetDays ?? 0 },
    { enabled: open, retry: 1 }
  )
  const email = query.data?.email
  return (
    <Dialog open={open} onOpenChange={o => !o && onClose()}>
      <DialogContent className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>{email?.subject ?? 'Reminder preview'}</DialogTitle>
        </DialogHeader>
        {query.isLoading && (
          <div className="space-y-2 py-4">
            <Skeleton height={16} width={240} />
            <Skeleton height={200} width="100%" />
          </div>
        )}
        {query.isError && (
          <div className="py-6 text-sm text-muted-foreground">
            {query.error.rawMessage || 'Couldn’t render this preview.'}
          </div>
        )}
        {email && (
          <div className="space-y-3">
            <div className="text-xs text-muted-foreground space-y-0.5">
              <div>
                <span className="font-medium">From:</span> {email.from}
              </div>
              {email.replyTo && (
                <div>
                  <span className="font-medium">Reply-To:</span> {email.replyTo}
                </div>
              )}
              {email.attachmentNames.length > 0 && (
                <div>
                  <span className="font-medium">Attachments:</span>{' '}
                  {email.attachmentNames.join(', ')}
                </div>
              )}
            </div>
            <TestBannerNote />
            <EmailHtmlFrame html={email.bodyHtml} />
            <div className="text-xs text-muted-foreground">
              Rendered from the current invoice data, {offsetPill(offsetDays ?? 0).toLowerCase()}.
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}

const SequenceLink = ({ sequence }: { sequence: SequenceRef }) => {
  const basePath = useBasePath()
  return (
    <span className="inline-flex items-center gap-1.5">
      <Link
        to={`${basePath}/settings/emails?view=reminders&sequence=${sequence.id}`}
        className="underline-offset-2 hover:underline"
      >
        {sequence.name}
      </Link>
      {sequence.archived ? (
        <Badge variant="ghost" size="sm">
          Archived
        </Badge>
      ) : (
        !sequence.enabled && (
          <Badge variant="ghost" size="sm">
            Disabled
          </Badge>
        )
      )}
    </span>
  )
}

const WhyThisSequence = ({ invoiceId, via }: { invoiceId: string; via: ReminderEnrolledVia }) => {
  const [open, setOpen] = useState(false)
  const query = useQuery(
    previewInvoiceReminderResolution,
    { invoiceId },
    { enabled: open, retry: 1 }
  )
  const rows = query.data?.rows ?? []
  const winnerIndex = rows.findIndex(r => r.wouldEnrol)

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex items-center gap-1 text-[12px] text-muted-foreground hover:text-foreground"
        >
          <CircleHelp size={12} />
          Why this sequence?
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 p-3 text-[13px]">
        <div className="mb-2 text-xs text-muted-foreground">
          {via === ReminderEnrolledVia.CUSTOMER_OVERRIDE
            ? 'This customer has a reminder override, which takes precedence over audiences.'
            : 'The first enabled sequence whose audience matches wins. The default catches everything else.'}
        </div>
        {query.isLoading && <Skeleton height={14} width={200} />}
        {query.isError && (
          <div className="text-muted-foreground">
            {query.error.rawMessage || 'Couldn’t evaluate the sequences.'}
          </div>
        )}
        {query.isSuccess && rows.length === 0 && (
          <div className="text-muted-foreground">No enabled sequence for this invoice.</div>
        )}
        <ul className="flex flex-col gap-1">
          {rows.map((row, i) => {
            const verdict = row.wouldEnrol
              ? 'Enrols'
              : row.matched
                ? i > winnerIndex && winnerIndex >= 0
                  ? 'Matches, claimed earlier'
                  : 'Matches'
                : 'No match'
            return (
              <li
                key={row.sequence?.id ?? i}
                className={cn(
                  'flex items-center justify-between gap-2',
                  !row.wouldEnrol && 'text-muted-foreground'
                )}
              >
                <span className="min-w-0 truncate">
                  {row.sequence?.name ?? 'Sequence'}
                  {row.isDefault && <span className="ml-1 text-xs">(default)</span>}
                </span>
                <Badge variant={row.wouldEnrol ? 'success' : 'outline'} size="sm">
                  {verdict}
                </Badge>
              </li>
            )
          })}
        </ul>
      </PopoverContent>
    </Popover>
  )
}

export const InvoiceRemindersCard = ({ invoiceId }: { invoiceId: string }) => {
  const queryClient = useQueryClient()
  const [showStopConfirmation, setShowStopConfirmation] = useState(false)
  const [sentPreviewId, setSentPreviewId] = useState<string | null>(null)
  const [previewOffset, setPreviewOffset] = useState<number | null>(null)

  const query = useQuery(
    getInvoiceReminders,
    { invoiceId },
    { enabled: Boolean(invoiceId), retry: 1 }
  )
  const state = query.data?.state

  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: getInvoiceReminders, cardinality: undefined }),
    })
  const onError = (fallback: string) => (err: { rawMessage: string }) =>
    toast.error(err.rawMessage || fallback)
  const onSuccess = (message: string) => () => {
    invalidate()
    toast.success(message)
  }

  const pauseMut = useMutation(pauseInvoiceReminders, {
    onSuccess: onSuccess('Reminders paused'),
    onError: onError('Could not pause reminders'),
  })
  const resumeMut = useMutation(resumeInvoiceReminders, {
    onSuccess: onSuccess('Reminders resumed'),
    onError: onError('Could not resume reminders'),
  })
  const stopMut = useMutation(stopInvoiceReminders, {
    onSuccess: () => {
      setShowStopConfirmation(false)
      onSuccess('Reminders stopped for this invoice')()
    },
    onError: onError('Could not stop reminders'),
  })
  const restartMut = useMutation(restartInvoiceReminders, {
    onSuccess: onSuccess('Reminders restarted'),
    onError: onError('Could not restart reminders'),
  })
  const sendNowMut = useMutation(sendInvoiceReminderNow, {
    onSuccess: onSuccess('Reminder queued'),
    onError: onError('Could not send the reminder'),
  })
  const busy =
    pauseMut.isPending ||
    resumeMut.isPending ||
    stopMut.isPending ||
    restartMut.isPending ||
    sendNowMut.isPending

  const pauseFor = (days: number) =>
    pauseMut.mutate({
      invoiceId,
      until: new Date(Date.now() + days * 24 * 60 * 60 * 1000).toISOString(),
    })

  const title = <div className="text-[15px] font-medium">Payment reminders</div>

  const body = (() => {
    if (query.isLoading) {
      return (
        <>
          {title}
          <Skeleton height={14} width={220} />
          <Skeleton height={14} width={160} />
        </>
      )
    }
    if (query.isError || !state) {
      return (
        <>
          {title}
          <div className="text-[13px] text-muted-foreground">
            {query.error?.rawMessage || 'Couldn’t load the reminder status.'}
          </div>
        </>
      )
    }

    const phase = phaseOf(state, new Date())
    const enrolled = Boolean(state.sequence)
    const isActive = phase === 'active' || phase === 'paused' || phase === 'held'
    const canRestart =
      !isActive && phase !== 'ended' && !RESTART_BLOCKED.has(state.notEligibleReason)
    const viaLabel = enrolledViaLabel(enrolled ? state.enrolledVia : state.wouldResolveVia)
    const shownSequence = state.sequence ?? state.wouldResolveTo

    return (
      <>
        <div className="flex items-center gap-2">
          {title}
          <StatusBadge state={state} phase={phase} />
        </div>

        {shownSequence && (
          <Row
            title={enrolled ? 'Sequence' : 'Would enrol in'}
            value={<SequenceLink sequence={shownSequence} />}
          />
        )}
        {viaLabel && (
          <Row
            title={enrolled ? 'Enrolled via' : 'Resolved via'}
            value={
              <span className="inline-flex flex-wrap items-center justify-end gap-2">
                {viaLabel}
                <WhyThisSequence
                  invoiceId={invoiceId}
                  via={enrolled ? state.enrolledVia : state.wouldResolveVia}
                />
              </span>
            }
          />
        )}
        {state.enrolledAt && (
          <Row title="Enrolled" value={parseAndFormatDateTime(state.enrolledAt)} />
        )}
        {phase === 'stopped' && state.stoppedAt && (
          <Row title="Stopped" value={parseAndFormatDateTime(state.stoppedAt)} />
        )}
        {phase === 'completed' && state.completedAt && (
          <Row title="Completed" value={parseAndFormatDateTime(state.completedAt)} />
        )}
        {phase === 'ended' && state.endedAt && (
          <Row title="Ended" value={parseAndFormatDateTime(state.endedAt)} />
        )}
        {isActive && state.nextReminderAt && (
          <Row
            title="Next reminder"
            value={
              phase === 'held' ? (
                <span className="text-muted-foreground">
                  {parseAndFormatDateTime(state.nextReminderAt)} (on hold)
                </span>
              ) : (
                `${parseAndFormatDateTime(state.nextReminderAt)}${state.sendWindow ? ` (${state.sendWindow})` : ''}`
              )
            }
          />
        )}

        {state.steps.length > 0 && (
          <div className="mt-2 flex flex-col gap-1">
            {!enrolled && (
              <div className="text-[11px] uppercase tracking-wide text-muted-foreground">
                If enrolled today
              </div>
            )}
            <StepTimeline
              steps={state.steps}
              muted={!enrolled}
              onViewSent={setSentPreviewId}
              onPreview={enrolled ? setPreviewOffset : undefined}
            />
          </div>
        )}

        <Gate perm="billing:invoices:manage">
          <div className="mt-2 flex flex-wrap items-center gap-2">
            {phase === 'active' && (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="outline" size="sm" disabled={busy}>
                    <Pause size={14} className="mr-2" />
                    Pause
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start">
                  {PAUSE_PRESETS.map(days => (
                    <DropdownMenuItem key={days} onClick={() => pauseFor(days)}>
                      For {days} days
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            )}
            {phase === 'paused' && (
              <Button
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={() => resumeMut.mutate({ invoiceId })}
              >
                <Play size={14} className="mr-2" />
                Resume
              </Button>
            )}
            {isActive && (
              <Button
                variant="outline"
                size="sm"
                className="text-destructive"
                disabled={busy}
                onClick={() => setShowStopConfirmation(true)}
              >
                <OctagonX size={14} className="mr-2" />
                Stop
              </Button>
            )}
            {canRestart && (
              <Button
                variant="outline"
                size="sm"
                disabled={busy}
                onClick={() => restartMut.mutate({ invoiceId })}
              >
                <RotateCcw size={14} className="mr-2" />
                Restart
              </Button>
            )}
            {enrolled && phase !== 'ended' && state.steps.length > 0 && (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button variant="outline" size="sm" disabled={busy}>
                    <Send size={14} className="mr-2" />
                    Send now
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="start">
                  {state.steps.map(step => (
                    <DropdownMenuItem
                      key={step.offsetDays}
                      onClick={() =>
                        sendNowMut.mutate({
                          invoiceId,
                          offsetDays: step.offsetDays,
                          requestId: crypto.randomUUID(),
                        })
                      }
                    >
                      {offsetPill(step.offsetDays)}
                      {step.status === InvoiceReminderStepStatus.SENT && (
                        <span className="ml-2 text-xs text-muted-foreground">sent</span>
                      )}
                    </DropdownMenuItem>
                  ))}
                </DropdownMenuContent>
              </DropdownMenu>
            )}
            {!enrolled && canRestart && (
              <span className="text-[12px] text-muted-foreground">
                Restart reminders first to send one now.
              </span>
            )}
          </div>
        </Gate>
      </>
    )
  })()

  return (
    <>
      <Separator className="-my-3" />
      <Flex direction="column" className="gap-2 p-6">
        {body}
      </Flex>

      <AlertDialog open={showStopConfirmation} onOpenChange={setShowStopConfirmation}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Stop reminders for this invoice</AlertDialogTitle>
            <AlertDialogDescription>
              No further payment reminders will be sent for this invoice. You can restart them
              manually later.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => stopMut.mutate({ invoiceId })}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Stop reminders
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {sentPreviewId && (
        <EmailPreviewDialog
          sentEmailId={sentPreviewId}
          open
          onOpenChange={open => !open && setSentPreviewId(null)}
        />
      )}
      <StepPreviewDialog
        invoiceId={invoiceId}
        offsetDays={previewOffset}
        onClose={() => setPreviewOffset(null)}
      />
    </>
  )
}
