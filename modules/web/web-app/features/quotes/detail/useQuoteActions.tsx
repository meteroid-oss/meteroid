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
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useState } from 'react'
import { toast } from 'sonner'

import { SendQuoteDialog } from '@/features/quotes/SendQuoteDialog'
import { CountersignDialog } from '@/features/quotes/detail/CountersignDialog'
import { DeclineReasonDialog } from '@/features/quotes/detail/DeclineReasonDialog'
import { QuotePreviewDialog } from '@/features/quotes/detail/QuotePreviewDialog'
import { SigningLinkDialog } from '@/features/quotes/detail/SigningLinkDialog'
import { rpcErrorMessage } from '@/features/quotes/detail/signers'
import { listEntityActivity } from '@/rpc/api/activity/v1/activity-ActivityService_connectquery'
import { DetailedQuote, QuoteCountersigner, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import {
  cancelQuote,
  convertQuoteToSubscription,
  declineQuote,
  generateQuotePortalToken,
  getQuote,
  publishQuote,
  sendQuote,
} from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'

/** Refreshes everything the quote service serves (the quote, lists, revisions, evidence) and its activity. */
export const useInvalidateQuote = () => {
  const queryClient = useQueryClient()
  return useCallback(
    () =>
      Promise.all([
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema: getQuote.parent, cardinality: undefined }),
        }),
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema: listEntityActivity, cardinality: undefined }),
        }),
      ]),
    [queryClient]
  )
}

type Dialog = 'send' | 'link' | 'preview' | 'countersign' | 'decline' | 'cancel'

export interface Remindee {
  name: string
  email: string
}

export const useQuoteActions = (quote: DetailedQuote) => {
  const quoteId = quote.quote?.id ?? ''
  const invalidate = useInvalidateQuote()
  const [dialog, setDialog] = useState<Dialog | null>(null)
  const [message, setMessage] = useState('')
  // Set when the email goes to one person only, e.g. a reminder from their signer row.
  const [sendTo, setSendTo] = useState<Remindee | undefined>()

  const options = { onSuccess: () => invalidate() }
  const publish = useMutation(publishQuote, options)
  const send = useMutation(sendQuote, options)
  const convert = useMutation(convertQuoteToSubscription, options)
  const cancel = useMutation(cancelQuote, options)
  const decline = useMutation(declineQuote, options)
  const token = useMutation(generateQuotePortalToken)

  const signingLink = async (email: string) => {
    try {
      const response = await token.mutateAsync({ quoteId, recipientEmail: email })
      return `${window.location.origin}/portal/quote?token=${response.token}`
    } catch (error) {
      toast.error(rpcErrorMessage(error, 'Failed to generate the signing link'))
      return undefined
    }
  }

  return {
    dialog,
    setDialog,
    message,
    setMessage,
    pending: {
      publish: publish.isPending,
      send: send.isPending,
      convert: convert.isPending,
      cancel: cancel.isPending,
      decline: decline.isPending,
      token: token.isPending,
    },
    invalidate,
    signingLink,
    sendTo,
    openSend: (person?: Remindee) => {
      setMessage('')
      setSendTo(person)
      setDialog('send')
    },
    publish: async () => {
      try {
        await publish.mutateAsync({ id: quoteId })
        toast.success('Quote published')
      } catch (error) {
        toast.error(rpcErrorMessage(error, 'Failed to publish the quote'))
      }
    },
    send: async () => {
      try {
        await send.mutateAsync({
          id: quoteId,
          message: message || undefined,
          recipientEmail: sendTo?.email,
        })
        toast.success(sendTo ? `Reminder sent to ${sendTo.name || sendTo.email}` : 'Quote sent')
        setDialog(null)
      } catch (error) {
        toast.error(rpcErrorMessage(error, 'Failed to send the quote'))
      }
    },
    convert: async () => {
      try {
        await convert.mutateAsync({ quoteId })
        toast.success('Quote converted to a subscription')
      } catch (error) {
        toast.error(rpcErrorMessage(error, 'Failed to convert the quote'))
      }
    },
    cancel: async () => {
      try {
        await cancel.mutateAsync({ id: quoteId })
        toast.success('Quote cancelled')
      } catch (error) {
        toast.error(rpcErrorMessage(error, 'Failed to cancel the quote'))
      }
    },
    decline: async (reason: string | undefined) => {
      try {
        await decline.mutateAsync({ id: quoteId, reason })
        toast.success('Quote marked as declined')
        setDialog(null)
      } catch (error) {
        toast.error(rpcErrorMessage(error, 'Failed to decline the quote'))
      }
    },
    copySigningLink: async (email: string) => {
      const url = await signingLink(email)
      if (!url) return
      await navigator.clipboard.writeText(url)
      toast.success(`Personal signing link of ${email} copied`, { id: 'copy' })
    },
  }
}

export type QuoteActions = ReturnType<typeof useQuoteActions>

interface DialogsProps {
  quote: DetailedQuote
  actions: QuoteActions
  countersigner?: QuoteCountersigner
}

export const QuoteActionDialogs = ({ quote, actions, countersigner }: DialogsProps) => {
  const { dialog, setDialog } = actions
  const openChange = (name: Dialog) => (open: boolean) => setDialog(open ? name : null)

  return (
    <>
      <SigningLinkDialog
        open={dialog === 'link'}
        onOpenChange={openChange('link')}
        defaultEmail={quote.quote?.recipients[0]?.email || quote.customer?.billingEmail || ''}
        isPending={actions.pending.token}
        generate={actions.signingLink}
      />

      <QuotePreviewDialog
        open={dialog === 'preview'}
        onOpenChange={openChange('preview')}
        quote={quote}
      />

      {countersigner && (
        <CountersignDialog
          open={dialog === 'countersign'}
          onOpenChange={openChange('countersign')}
          quote={quote}
          countersigner={countersigner}
          onSigned={() => void actions.invalidate()}
        />
      )}

      <DeclineReasonDialog
        open={dialog === 'decline'}
        onOpenChange={openChange('decline')}
        title="Mark as declined"
        confirmLabel="Mark as declined"
        isPending={actions.pending.decline}
        onConfirm={actions.decline}
      />

      <SendQuoteDialog
        open={dialog === 'send'}
        onOpenChange={open => {
          setDialog(open ? 'send' : null)
          if (!open) actions.setMessage('')
        }}
        quoteNumber={quote.quote?.quoteNumber}
        recipients={actions.sendTo ? [actions.sendTo] : quote.quote?.recipients}
        reminder={quote.quote?.status === QuoteStatus.PENDING}
        customMessage={actions.message}
        onCustomMessageChange={actions.setMessage}
        onConfirm={actions.send}
        isPending={actions.pending.send}
        emailEnabled={quote.quoteEmailEnabled}
      />

      <AlertDialog open={dialog === 'cancel'} onOpenChange={openChange('cancel')}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Cancel quote {quote.quote?.quoteNumber}?</AlertDialogTitle>
            <AlertDialogDescription>This cannot be undone.</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep the quote</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={actions.pending.cancel}
              onClick={() => void actions.cancel()}
            >
              Cancel quote
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
