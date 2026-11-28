import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Alert,
  AlertDescription,
  Button,
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  cn,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Eye, Pen, RefreshCw } from 'lucide-react'
import { FC, useMemo, useState } from 'react'
import { toast } from 'sonner'

import { PortalTestModeBanner } from '@/components/PortalTestModeBanner'
import { QuoteSignerSlot } from '@/features/quotes/QuoteViewSigners'
import { recurringSummary, termSummary } from '@/features/quotes/QuoteViewTotals'
import { DeclineReasonDialog } from '@/features/quotes/detail/DeclineReasonDialog'
import {
  DEFAULT_CONSENT_TEXT,
  hasSignedAs,
  rpcErrorMessage,
  sameEmail,
} from '@/features/quotes/detail/signers'
import { QuestionIntent, QuestionsPanel } from '@/features/quotes/portal/QuestionsPanel'
import { QuoteActionBar, QuoteOutcome } from '@/features/quotes/portal/QuoteActionBar'
import { QuotePortalTopBar } from '@/features/quotes/portal/QuotePortalTopBar'
import { SignQuoteDialog } from '@/features/quotes/portal/SignQuoteDialog'
import { quoteBrandStyle } from '@/features/quotes/portal/branding'
import { useMediaQuery } from '@/hooks/useMediaQuery'
import { env } from '@/lib/env'
import { QuoteSignatureLevel, QuoteSignerRole, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import { QuotePortalDetails } from '@/rpc/portal/quotes/v1/models_pb'
import {
  declineQuote,
  getQuotePortal,
} from '@/rpc/portal/quotes/v1/quotes-PortalQuoteService_connectquery'
import { SignQuoteResponse } from '@/rpc/portal/quotes/v1/quotes_pb'
import { formatDateOnly, parseDateField } from '@/utils/date'

import { QuoteView } from './QuoteView'

export interface QuotePortalViewProps {
  quoteData: QuotePortalDetails
  /** Read-only rendering for the seller: nothing is signed, declined or fetched with a token. */
  preview?: boolean
}

const DAY_MS = 24 * 60 * 60 * 1000

const QuotePortalView: FC<QuotePortalViewProps> = ({ quoteData, preview = false }) => {
  const isDesktop = useMediaQuery('(min-width: 1024px)')
  const comments = useMemo(() => quoteData.comments ?? [], [quoteData.comments])

  const [panelOpen, setPanelOpen] = useState(() => isDesktop && comments.length > 0)
  const [intent, setIntent] = useState<QuestionIntent>('question')
  const [anchor, setAnchor] = useState<string>()
  const [focusRequest, setFocusRequest] = useState(0)
  const [showSignDialog, setShowSignDialog] = useState(false)
  const [showDeclineDialog, setShowDeclineDialog] = useState(false)
  const [codeVerified, setCodeVerified] = useState(quoteData.codeVerified)
  const [outcome, setOutcome] = useState<SignQuoteResponse>()
  const [stale, setStale] = useState(false)

  const brandStyle = useMemo(() => quoteBrandStyle(quoteData, preview), [quoteData, preview])

  const queryClient = useQueryClient()
  const refresh = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: getQuotePortal.parent, cardinality: undefined }),
    })

  const declineMutation = useMutation(declineQuote)

  const quote = quoteData.quote
  const customer = quoteData.customer
  const signatures = quoteData.signatures
  const countersigners = quoteData.countersigners
  const sellerName = quoteData.entity?.legalName || 'The seller'

  const role = quoteData.currentSignerRole
  const isCountersigner = role === QuoteSignerRole.SIGNER_COUNTERSIGNER
  const isVerifiedLevel = quote?.signatureLevel === QuoteSignatureLevel.VERIFIED
  const consentText = quote?.consentText || quoteData.defaultConsentText || DEFAULT_CONSENT_TEXT

  const expiresAt = quote?.expiresAt ? parseDateField(quote.expiresAt) : undefined
  const now = Date.now()
  const isExpired =
    quote?.status === QuoteStatus.EXPIRED || (expiresAt !== undefined && expiresAt.getTime() < now)
  const daysLeft = expiresAt ? Math.floor((expiresAt.getTime() - now) / DAY_MS) : undefined
  const isDraft = quote?.status === QuoteStatus.DRAFT
  const isPending = quote?.status === QuoteStatus.PENDING
  const isCancelled = quote?.status === QuoteStatus.CANCELLED
  const isAccepted = quote?.status === QuoteStatus.ACCEPTED || Boolean(outcome?.accepted)
  const isDeclined = quote?.status === QuoteStatus.DECLINED

  const currentUserEmail = quoteData.currentRecipientEmail || customer?.billingEmail || ''
  const currentUserName = quoteData.currentRecipientName || customer?.name || ''
  const hasCurrentUserSigned = Boolean(outcome) || hasSignedAs(signatures, currentUserEmail, role)
  const canSign = isPending && !isExpired && !hasCurrentUserSigned && !isAccepted
  const allCustomersSigned = quoteData.recipients.every(r => r.hasSigned)
  const awaitingCountersignature =
    outcome?.awaitingCountersignature ??
    (!isCountersigner && allCustomersSigned && countersigners.some(c => !c.hasSigned))

  const discussionAvailable =
    !isDraft &&
    (quoteData.discussionEnabled || quoteData.changeRequestsEnabled || comments.length > 0)
  const canRequestChanges = quoteData.changeRequestsEnabled && !isCountersigner
  const canDecline = quoteData.declineEnabled && !isCountersigner

  const commentCountByLine = useMemo(() => {
    const counts: Record<string, number> = {}
    for (const comment of comments) {
      if (comment.anchor) counts[comment.anchor] = (counts[comment.anchor] ?? 0) + 1
    }
    return counts
  }, [comments])

  const signers = useMemo((): QuoteSignerSlot[] => {
    const slots: QuoteSignerSlot[] = quoteData.recipients.map(recipient => {
      const signature = signatures.find(
        s =>
          s.signerRole === QuoteSignerRole.SIGNER_CUSTOMER &&
          sameEmail(s.signedByEmail, recipient.email)
      )
      const current = !isCountersigner && sameEmail(recipient.email, currentUserEmail)
      return {
        key: `customer:${recipient.email}`,
        role: QuoteSignerRole.SIGNER_CUSTOMER,
        name: recipient.name || recipient.email,
        signed: recipient.hasSigned || Boolean(signature) || (current && hasCurrentUserSigned),
        signedAt: signature?.signedAt,
        current,
      }
    })
    if (!isCountersigner && currentUserEmail && !slots.some(s => s.current)) {
      slots.unshift({
        key: `customer:${currentUserEmail}`,
        role: QuoteSignerRole.SIGNER_CUSTOMER,
        name: currentUserName || currentUserEmail,
        signed: hasCurrentUserSigned,
        current: true,
      })
    }
    countersigners.forEach((countersigner, index) => {
      const signature = signatures.find(
        s =>
          s.signerRole === QuoteSignerRole.SIGNER_COUNTERSIGNER &&
          s.signedByName === countersigner.name
      )
      const current = isCountersigner && countersigner.isViewer
      slots.push({
        key: `countersigner:${index}`,
        role: QuoteSignerRole.SIGNER_COUNTERSIGNER,
        name: countersigner.name,
        signed: countersigner.hasSigned || (current && hasCurrentUserSigned),
        signedAt: signature?.signedAt,
        current,
      })
    })
    return slots
  }, [
    quoteData.recipients,
    signatures,
    countersigners,
    isCountersigner,
    currentUserEmail,
    currentUserName,
    hasCurrentUserSigned,
  ])

  const totals = quoteData.totals
  const recurring = recurringSummary(totals)
  const term = termSummary(quote)
  const summary = recurring ? [recurring, term].filter(Boolean).join(' · ') : undefined
  const outcomeState: QuoteOutcome | undefined = isDeclined
    ? { kind: 'declined', reason: quote?.declineReason }
    : isAccepted
      ? { kind: 'accepted', signedPdfUrl: preview ? undefined : quoteData.pdfUrl }
      : hasCurrentUserSigned
        ? { kind: 'signed', awaitingCountersignature }
        : isCancelled
          ? { kind: 'cancelled' }
          : isExpired
            ? { kind: 'expired', on: quote?.expiresAt && formatDateOnly(quote.expiresAt) }
            : isDraft
              ? { kind: 'draft' }
              : undefined

  const openPanel = (nextIntent: QuestionIntent, nextAnchor?: string) => {
    setIntent(nextIntent)
    if (nextAnchor !== undefined) setAnchor(nextAnchor)
    setPanelOpen(true)
    // On phones, focusing the message field pops the keyboard over the thread being opened.
    if (isDesktop || nextIntent === 'change' || nextAnchor !== undefined) {
      setFocusRequest(n => n + 1)
    }
  }

  const closePanel = () => {
    setPanelOpen(false)
    setFocusRequest(0)
  }

  const openSign = () => setShowSignDialog(true)

  const handleDecline = async (reason: string | undefined) => {
    try {
      await declineMutation.mutateAsync({ reason })
      setShowDeclineDialog(false)
      toast.success('You declined the quote')
      void refresh()
    } catch (error) {
      toast.error(rpcErrorMessage(error, 'The quote could not be declined'))
    }
  }

  const reload = async () => {
    await refresh()
    setStale(false)
  }

  const logoUrl =
    quoteData.logoUrl ??
    (quoteData.entity?.logoAttachmentId
      ? `${env.meteroidRestApiUri}/files/v1/logo/${quoteData.entity.logoAttachmentId}`
      : undefined)

  const panel = (onClose?: () => void) => (
    <QuestionsPanel
      quoteData={quoteData}
      preview={preview}
      currentUserEmail={currentUserEmail}
      intent={intent}
      anchor={anchor}
      onAnchorChange={setAnchor}
      focusRequest={focusRequest}
      onClose={onClose}
    />
  )

  return (
    <div
      className={cn(
        'flex flex-col bg-muted/40 text-foreground dark:bg-background',
        preview ? 'relative' : 'min-h-screen'
      )}
      style={brandStyle}
    >
      <PortalTestModeBanner active={!preview && quoteData.testMode} />
      {preview && (
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center overflow-hidden"
        >
          <span className="-rotate-30 select-none text-8xl font-bold uppercase tracking-widest text-muted-foreground/10">
            Preview
          </span>
        </div>
      )}

      <QuotePortalTopBar
        sellerName={sellerName}
        logoUrl={logoUrl}
        quoteNumber={quote?.quoteNumber}
        customerName={customer?.name}
        viewerName={currentUserName}
        viewerEmail={currentUserEmail}
        status={quote?.status}
        pdfUrl={preview || isAccepted ? undefined : quoteData.pdfUrl}
        expired={isExpired && !isAccepted && !isDeclined}
        daysLeft={daysLeft}
        questions={
          discussionAvailable
            ? {
                count: comments.length,
                open: panelOpen,
                onToggle: () => (panelOpen ? closePanel() : openPanel('question')),
              }
            : undefined
        }
      />

      <div className="mx-auto flex w-full max-w-[1320px] flex-1 items-start gap-6 px-4 py-4 sm:px-6 sm:py-6">
        <main className="min-w-0 flex-1 space-y-4">
          {preview && (
            <Alert>
              <Eye className="h-4 w-4" />
              <AlertDescription>Preview as {currentUserName || currentUserEmail}</AlertDescription>
            </Alert>
          )}

          {stale && (
            <Alert variant="warning">
              <RefreshCw className="h-4 w-4" />
              <AlertDescription className="flex flex-wrap items-center justify-between gap-3">
                This quote has changed.
                <Button size="sm" variant="outline" onClick={() => void reload()}>
                  Reload
                </Button>
              </AlertDescription>
            </Alert>
          )}

          {isCountersigner && isPending && !hasCurrentUserSigned && (
            <Alert>
              <Pen className="h-4 w-4" />
              <AlertDescription>
                {customer?.name || 'The customer'} signed · your countersignature is requested
              </AlertDescription>
            </Alert>
          )}

          {summary && (
            <div className="rounded-lg border border-border bg-card p-4 sm:hidden">
              <p className="text-xl font-bold text-foreground">{recurring}</p>
              {term && <p className="text-sm text-muted-foreground">{term}</p>}
            </div>
          )}

          {quote && (
            <QuoteView
              quote={{
                components: quoteData.components,
                addOns: quoteData.addOns,
                couponTerms: quoteData.couponTerms,
                priceSchedule: quoteData.priceSchedule,
                contractTerm: quoteData.contractTerm,
                quote,
                customer,
                invoicingEntity: quoteData.entity,
              }}
              mode="portal"
              minimumCommitment={quoteData.minimumCommitment}
              totals={totals}
              commentCountByLine={commentCountByLine}
              onLineSelect={
                discussionAvailable ? lineName => openPanel('question', lineName) : undefined
              }
              selectedLine={panelOpen ? anchor : undefined}
              signers={signers}
              onSign={canSign ? openSign : undefined}
              signDisabled={preview || stale}
            />
          )}

          {quoteData.showPoweredBy && (
            <p className="flex items-center justify-center gap-1.5 pt-2 text-xs text-muted-foreground">
              Powered by
              <a
                href="https://meteroid.com/?utm_source=quote-portal"
                target="_blank"
                rel="noopener noreferrer"
                className="font-semibold text-foreground hover:underline"
              >
                Meteroid
              </a>
            </p>
          )}
        </main>

        {isDesktop && discussionAvailable && panelOpen && (
          <aside
            aria-label="Questions and changes"
            className="sticky top-4 flex max-h-[calc(100vh-8rem)] w-[340px] shrink-0 flex-col overflow-hidden rounded-lg border border-border bg-card"
          >
            {panel(closePanel)}
          </aside>
        )}
      </div>

      <QuoteActionBar
        summary={summary}
        outcome={outcomeState}
        actions={
          canSign
            ? {
                signLabel: isCountersigner ? 'Review & countersign' : 'Review & sign',
                onSign: openSign,
                onRequestChanges: canRequestChanges ? () => openPanel('change') : undefined,
                onDecline: canDecline ? () => setShowDeclineDialog(true) : undefined,
                disabled: preview,
                signDisabled: stale,
              }
            : undefined
        }
      />

      {!isDesktop && discussionAvailable && (
        <Sheet open={panelOpen} onOpenChange={open => (open ? setPanelOpen(true) : closePanel())}>
          <SheetContent
            side="right"
            size="small"
            style={brandStyle}
            aria-describedby={undefined}
            className="flex flex-col gap-0 p-0"
          >
            <SheetHeader className="border-b border-border px-4 py-3 pr-12">
              <SheetTitle className="text-base">Questions & changes</SheetTitle>
            </SheetHeader>
            {panel()}
          </SheetContent>
        </Sheet>
      )}

      {!preview && (
        <>
          <SignQuoteDialog
            open={showSignDialog}
            onOpenChange={setShowSignDialog}
            style={brandStyle}
            quoteNumber={quote?.quoteNumber}
            summary={summary}
            recipientEmail={currentUserEmail}
            recipientName={currentUserName}
            consentText={consentText}
            documentHash={quote?.documentHash}
            isCountersignature={isCountersigner}
            verification={{
              required: isVerifiedLevel,
              verified: codeVerified,
              onVerified: () => setCodeVerified(true),
            }}
            purchaseOrder={{
              enabled: quoteData.purchaseOrderEnabled,
              current: quote?.purchaseOrder,
            }}
            onPurchaseOrderSaved={() => void refresh()}
            onSigned={(response: SignQuoteResponse) => {
              setOutcome(response)
              void refresh()
            }}
            onStale={() => setStale(true)}
          />
          <DeclineReasonDialog
            open={showDeclineDialog}
            onOpenChange={setShowDeclineDialog}
            title="Decline quote"
            description="Let the seller know you are not going ahead with this quote. You can say why."
            confirmLabel="Decline quote"
            isPending={declineMutation.isPending}
            onConfirm={handleDecline}
          />
        </>
      )}
    </div>
  )
}

export default QuotePortalView
