import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { zodResolver } from '@hookform/resolvers/zod'
import {
  Alert,
  AlertDescription,
  Badge,
  Button,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Textarea,
  cn,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { MessageSquareWarning, X } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { useForm } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'

import { CommentPin } from '@/features/quotes/detail/QuoteLinesTable'
import { rpcErrorMessage, sameEmail } from '@/features/quotes/detail/signers'
import { parseAndFormatDateTime, parseUtc } from '@/lib/utils/date'
import { QuoteComment } from '@/rpc/api/quotes/v1/models_pb'
import { QuotePortalDetails } from '@/rpc/portal/quotes/v1/models_pb'
import {
  addQuoteComment,
  getQuotePortal,
} from '@/rpc/portal/quotes/v1/quotes-PortalQuoteService_connectquery'

export type QuestionIntent = 'question' | 'change'

const formatCommentTime = (value: string) => parseAndFormatDateTime(parseUtc(value).toISOString())

const messageSchema = z.object({
  body: z
    .string()
    .trim()
    .min(1, 'Write a message')
    .max(10000, 'The message is limited to 10,000 characters'),
})

type MessageForm = z.infer<typeof messageSchema>

interface Props {
  quoteData: QuotePortalDetails
  preview: boolean
  currentUserEmail: string
  intent: QuestionIntent
  /** The line the next message is about. */
  anchor?: string
  onAnchorChange: (anchor: string | undefined) => void
  /** Incremented to move the focus to the message field. */
  focusRequest: number
  /** Desktop panel: shows the title and a close button. */
  onClose?: () => void
}

const CommentItem = ({
  comment,
  own,
  highlighted,
  onAnchorSelect,
}: {
  comment: QuoteComment
  own: boolean
  highlighted: boolean
  onAnchorSelect: (anchor: string) => void
}) => (
  <li
    className={cn(
      'rounded-lg border border-border p-3 text-sm',
      highlighted ? 'bg-muted ring-1 ring-inset ring-brand' : own ? 'bg-muted/50' : 'bg-card'
    )}
  >
    {comment.anchor && (
      <button
        type="button"
        onClick={() => onAnchorSelect(comment.anchor!)}
        className="mb-1.5 inline-flex max-w-full items-center gap-1 truncate rounded-sm text-xs text-muted-foreground hover:text-foreground hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        On {comment.anchor}
      </button>
    )}
    <div className="flex flex-wrap items-center gap-x-1.5 text-xs text-muted-foreground">
      <span className="font-medium text-foreground">{own ? 'You' : comment.authorName}</span>
      <span>· {formatCommentTime(comment.createdAt)}</span>
    </div>
    <p className="mt-1 whitespace-pre-wrap break-words text-foreground">{comment.body}</p>
    {comment.isChangeRequest && (
      <Badge variant={comment.resolvedAt ? 'success' : 'warning'} size="sm" className="mt-2">
        {comment.resolvedAt ? 'Change resolved' : 'Change requested'}
      </Badge>
    )}
  </li>
)

/** Messages with the seller and change requests, anchored to a line when one was picked. */
export const QuestionsPanel = ({
  quoteData,
  preview,
  currentUserEmail,
  intent,
  anchor,
  onAnchorChange,
  focusRequest,
  onClose,
}: Props) => {
  const queryClient = useQueryClient()
  const textareaRef = useRef<HTMLTextAreaElement | null>(null)
  const comments = quoteData.comments ?? []
  const canAsk = quoteData.discussionEnabled
  const canRequestChanges = quoteData.changeRequestsEnabled
  const canWrite = canAsk || canRequestChanges

  const form = useForm<z.input<typeof messageSchema>, unknown, MessageForm>({
    resolver: zodResolver(messageSchema),
    defaultValues: { body: '' },
  })

  useEffect(() => {
    if (focusRequest > 0) textareaRef.current?.focus()
  }, [focusRequest])

  const add = useMutation(addQuoteComment, {
    onSuccess: (_, request) => {
      form.reset({ body: '' })
      onAnchorChange(undefined)
      toast.success(request.requestChanges ? 'Change requested' : 'Message sent')
      void queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: getQuotePortal.parent, cardinality: undefined }),
      })
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The message could not be sent')),
  })

  const send = (requestChanges: boolean) =>
    form.handleSubmit(values => add.mutate({ body: values.body, requestChanges, anchor }))

  const disabled = preview || add.isPending
  const changeFirst = intent === 'change' && canRequestChanges
  const anchorCount = anchor ? comments.filter(c => c.anchor === anchor).length : 0

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {onClose && (
        <div className="flex items-center justify-between gap-2 border-b border-border px-4 py-3">
          <h2 className="text-sm font-semibold text-foreground">Questions & changes</h2>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            onClick={onClose}
            aria-label="Close questions and changes"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      )}

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4">
        {quoteData.quote?.changesRequestedAt && (
          <Alert>
            <MessageSquareWarning className="h-4 w-4" />
            <AlertDescription>You requested changes</AlertDescription>
          </Alert>
        )}

        {comments.length === 0 ? (
          <p className="text-sm text-muted-foreground">No messages</p>
        ) : (
          <ol className="space-y-2" aria-label="Messages">
            {comments.map(comment => (
              <CommentItem
                key={comment.id}
                comment={comment}
                own={comment.fromCustomer && sameEmail(comment.authorEmail, currentUserEmail)}
                highlighted={anchor !== undefined && comment.anchor === anchor}
                onAnchorSelect={onAnchorChange}
              />
            ))}
          </ol>
        )}
      </div>

      {canWrite && (
        <div className="border-t border-border p-4">
          <Form {...form}>
            <form
              onSubmit={e => {
                e.preventDefault()
                void send(changeFirst || !canAsk)()
              }}
              className="space-y-3"
            >
              {anchor && (
                <div className="flex items-center gap-1">
                  <span className="inline-flex max-w-full items-center gap-1.5 rounded-full border border-border bg-muted px-2 py-0.5 text-xs text-foreground">
                    {anchorCount > 0 && (
                      <CommentPin count={anchorCount} className="h-4 min-w-4 text-[10px]" />
                    )}
                    <span className="truncate">On {anchor}</span>
                    <button
                      type="button"
                      onClick={() => onAnchorChange(undefined)}
                      aria-label={`Stop referring to ${anchor}`}
                      className="rounded-full text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </span>
                </div>
              )}
              <FormField
                control={form.control}
                name="body"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{changeFirst ? 'Change request' : 'Message'}</FormLabel>
                    <FormControl>
                      <Textarea
                        {...field}
                        ref={element => {
                          field.ref(element)
                          textareaRef.current = element
                        }}
                        rows={3}
                        maxLength={10000}
                        disabled={preview}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <div className="flex flex-wrap gap-2">
                {canAsk && (
                  <Button
                    type="button"
                    size="sm"
                    variant={changeFirst ? 'outline' : 'brand'}
                    disabled={disabled}
                    onClick={() => void send(false)()}
                  >
                    Ask a question
                  </Button>
                )}
                {canRequestChanges && (
                  <Button
                    type="button"
                    size="sm"
                    variant={changeFirst || !canAsk ? 'brand' : 'outline'}
                    disabled={disabled}
                    onClick={() => void send(true)()}
                  >
                    Request a change
                  </Button>
                )}
              </div>
            </form>
          </Form>
        </div>
      )}
    </div>
  )
}
