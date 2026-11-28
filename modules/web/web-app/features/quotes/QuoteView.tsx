import { Card, CardContent } from '@md/ui'
import { FC, ReactNode } from 'react'

import { Slot } from '@/edition'
import { MinimumCommitmentDisplay, productScopeOptions } from '@/features/billing/minimumCommitment'
import { AddressLinesCompact } from '@/features/customers/cards/address/AddressCard'
import { QuoteContractValueSection, QuotePriceSchedule } from '@/features/quotes/QuotePriceSchedule'
import { QuoteDiscounts } from '@/features/quotes/QuoteViewDiscounts'
import {
  QuoteSignatureBlocks,
  QuoteSignerSlot,
  signerSlotsFromQuote,
} from '@/features/quotes/QuoteViewSigners'
import { QuoteTotalsRows } from '@/features/quotes/QuoteViewTotals'
import { QuoteLine, QuoteLinesTable, addOnLine } from '@/features/quotes/detail/QuoteLinesTable'
import { QuoteSections } from '@/features/quotes/detail/QuoteMarkdown'
import { quoteTrialLabel } from '@/features/quotes/quoteTrial'
import { env } from '@/lib/env'
import { Customer } from '@/rpc/api/customers/v1/models_pb'
import { InvoicingEntity, InvoicingEntityPublic } from '@/rpc/api/invoicingentities/v1/models_pb'
import { MinimumCommitment } from '@/rpc/api/plans/v1/models_pb'
import {
  Quote,
  QuoteAddOn,
  QuoteComponent,
  QuoteCouponTerms,
  QuotePriceStep,
  QuoteContractTerm,
  QuoteSectionPlacement,
  QuoteSignature,
  QuoteTotals,
  QuoteTrialEndAction,
} from '@/rpc/api/quotes/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

export interface QuoteViewProps {
  quote: {
    quote?: Quote
    invoicingEntity?: InvoicingEntity | InvoicingEntityPublic
    customer?: Customer
    components?: QuoteComponent[]
    addOns?: QuoteAddOn[]
    couponTerms?: QuoteCouponTerms[]
    priceSchedule?: QuotePriceStep[]
    signatures?: QuoteSignature[]
    contractTerm?: QuoteContractTerm
  }
  mode?: 'preview' | 'detailed' | 'portal'
  className?: string
  subscriptionComponents?: QuoteLine[]
  /** Effective minimum-commitment term to show on the document (override or plan default). */
  minimumCommitment?: MinimumCommitment
  totals?: QuoteTotals
  /** Comment count per line, keyed by the line's name (a comment's anchor). */
  commentCountByLine?: Record<string, number>
  /** Makes each line name a button, e.g. to discuss that line. */
  onLineSelect?: (lineName: string) => void
  selectedLine?: string
  /** Replaces the signers derived from the quote's recipients, countersigners and signatures. */
  signers?: QuoteSignerSlot[]
  /** The current signer's unsigned block becomes the call to sign. */
  onSign?: () => void
  signDisabled?: boolean
}

const LegalIdentifiers = ({
  vatNumber,
  legalNumber,
}: {
  vatNumber?: string
  legalNumber?: string
}) => (
  <>
    {vatNumber && <p>VAT number: {vatNumber}</p>}
    {legalNumber && <p>Registration number: {legalNumber}</p>}
  </>
)

const Section = ({ title, children }: { title: string; children: ReactNode }) => (
  <section className="mb-8">
    <h3 className="mb-3 font-semibold text-foreground">{title}</h3>
    {children}
  </section>
)

const DetailRow = ({ label, value }: { label: string; value: ReactNode }) => (
  <div className="flex justify-between gap-4 text-sm">
    <span className="text-muted-foreground">{label}</span>
    <span className="text-right text-foreground">{value}</span>
  </div>
)

export const QuoteView: FC<QuoteViewProps> = ({
  quote,
  mode = 'detailed',
  className = '',
  subscriptionComponents,
  minimumCommitment,
  totals,
  commentCountByLine,
  onLineSelect,
  selectedLine,
  signers,
  onSign,
  signDisabled,
}) => {
  const q = quote.quote
  const entity = quote.invoicingEntity

  const renderQuoteHeader = () => (
    <div className="mb-8 flex flex-col-reverse items-start justify-between gap-4 sm:flex-row">
      <div className="min-w-0">
        <h1 className="text-2xl font-bold text-foreground">Quote</h1>
        <p className="text-lg font-medium text-foreground">{q?.quoteNumber || 'DRAFT'}</p>
        <p className="mt-1 text-sm text-muted-foreground">
          {q?.createdAt ? parseAndFormatDate(q.createdAt) : new Date().toLocaleDateString()}
        </p>
        {q?.expiresAt && (
          <p className="mt-1 text-sm text-muted-foreground">
            Expires: <span className="text-foreground">{parseAndFormatDate(q.expiresAt)}</span>
          </p>
        )}
        {q?.purchaseOrder && (
          <p className="mt-1 text-sm text-muted-foreground">
            Purchase order: <span className="text-foreground">{q.purchaseOrder}</span>
          </p>
        )}
      </div>
      {entity?.logoAttachmentId && (
        <div className="h-12 w-32 shrink-0 overflow-hidden rounded-lg">
          <img
            src={env.meteroidRestApiUri + '/files/v1/logo/' + entity.logoAttachmentId}
            alt={entity.legalName ? `${entity.legalName} logo` : 'Company logo'}
            className="h-full w-full object-contain object-left sm:object-right"
          />
        </div>
      )}
    </div>
  )

  const renderFromToSection = () => (
    <div className="mb-8 grid grid-cols-1 gap-6 sm:grid-cols-2 sm:gap-8">
      <div>
        <h3 className="mb-2 font-semibold text-foreground">From:</h3>
        <div className="text-sm text-muted-foreground">
          {entity ? (
            <>
              <p className="font-medium text-foreground">{entity.legalName}</p>
              {entity.city && (
                <AddressLinesCompact
                  address={{
                    city: entity.city,
                    country: entity.country,
                    line1: entity.addressLine1,
                    line2: entity.addressLine2,
                    state: entity.state,
                    zipCode: entity.zipCode,
                  }}
                />
              )}
              <LegalIdentifiers vatNumber={entity.vatNumber} legalNumber={entity.legalNumber} />
            </>
          ) : (
            <p className="font-medium text-foreground">Your Company</p>
          )}
        </div>
      </div>
      <div>
        <h3 className="mb-2 font-semibold text-foreground">To:</h3>
        <div className="break-words text-sm text-muted-foreground">
          {quote.customer ? (
            <>
              <p className="font-medium text-foreground">{quote.customer.name}</p>
              <p>{quote.customer.billingEmail}</p>
              {quote.customer.billingAddress && (
                <AddressLinesCompact address={quote.customer.billingAddress} />
              )}
              <LegalIdentifiers
                vatNumber={quote.customer.vatNumber}
                legalNumber={quote.customer.legalNumber}
              />
            </>
          ) : q?.customerId ? (
            <p className="font-medium text-foreground">Customer ID: {q.customerId}</p>
          ) : (
            <p className="font-medium text-muted-foreground">No customer selected</p>
          )}
        </div>
      </div>
    </div>
  )

  const lineProps = { commentCountByLine, onLineSelect, selectedLine }

  const renderPricing = () => {
    if (!q?.currency) {
      return (
        <div className="mb-8">
          <p className="font-medium text-muted-foreground">No currency selected</p>
        </div>
      )
    }
    const lines: QuoteLine[] =
      mode === 'preview' && subscriptionComponents ? subscriptionComponents : quote.components || []
    const addOns = quote.addOns ?? []

    return (
      <>
        <QuoteLinesTable
          lines={lines}
          currency={q.currency}
          displayOptions={q.displayOptions}
          {...lineProps}
        />
        {addOns.length > 0 && (
          <QuoteLinesTable
            lines={addOns.map(addOnLine)}
            currency={q.currency}
            label="Add-ons"
            displayOptions={q.displayOptions}
            hideWhenEmpty
            {...lineProps}
          />
        )}
        <QuoteDiscounts coupons={quote.couponTerms ?? []} />
        {totals && (
          <QuoteTotalsRows
            totals={totals}
            showListPrice={Boolean(q.displayOptions?.showListPrice)}
          />
        )}
        <QuotePriceSchedule
          steps={quote.priceSchedule ?? []}
          contract={totals?.contract}
          currency={q.currency}
        />
        <QuoteContractValueSection value={totals?.contract} currency={q.currency} />
      </>
    )
  }

  const renderSubscriptionDetails = () => {
    if (!q) return null

    return (
      <Section title="Subscription Details">
        <div className="grid gap-x-8 gap-y-2 rounded-lg bg-muted/50 p-4 lg:grid-cols-2">
          <DetailRow
            label="Start Date:"
            value={q.startDate ? parseAndFormatDate(q.startDate) : 'Immediately'}
          />
          {q.endDate && <DetailRow label="End Date:" value={parseAndFormatDate(q.endDate)} />}
          {q.billingStartDate && (
            <DetailRow label="Billing Start Date:" value={parseAndFormatDate(q.billingStartDate)} />
          )}
          {q.trialDuration !== undefined && q.trialDuration > 0 && (
            <DetailRow
              label="Trial Duration:"
              value={quoteTrialLabel({
                days: q.trialDuration,
                free: q.trialFree !== false,
                manual: q.trialEndAction === QuoteTrialEndAction.MANUAL,
              })}
            />
          )}
          {!!q.trialDuration && q.trialPlanName && (
            <DetailRow label="Features of:" value={q.trialPlanName} />
          )}
          {q.billingDayAnchor && (
            <DetailRow label="Billing Day:" value={`Day ${q.billingDayAnchor} of each month`} />
          )}
          <DetailRow
            label="Payment Terms:"
            value={q.netTerms === 0 ? 'Due on receipt' : `Net ${q.netTerms} days`}
          />
        </div>
      </Section>
    )
  }

  const renderContractTerms = () => {
    const term = quote.contractTerm
    if ((!minimumCommitment || !q?.currency) && !term) return null

    // Scope is keyed by product; resolve product names from the quote's components.
    const componentOptions = productScopeOptions(quote.components || [])

    return (
      <Section title="Contract Terms">
        <div className="flex flex-col gap-3 rounded-lg bg-muted/50 p-4">
          {term && <ContractTermRows term={term} />}
          {minimumCommitment && q?.currency && (
            <MinimumCommitmentDisplay
              commitment={minimumCommitment}
              currency={q.currency}
              components={componentOptions}
            />
          )}
        </div>
      </Section>
    )
  }

  const renderSignatures = () => (
    <section className="border-t border-border pt-8">
      <h3 className="mb-4 font-semibold text-foreground">Signatures</h3>
      <QuoteSignatureBlocks
        signers={signers ?? signerSlotsFromQuote(q, quote.signatures ?? [])}
        onSign={onSign}
        signDisabled={signDisabled}
      />
    </section>
  )

  return (
    <Card className={className}>
      <CardContent className="p-5 sm:p-8">
        {renderQuoteHeader()}
        {renderFromToSection()}
        {q?.overview && (
          <section className="mb-8">
            <h3 className="mb-2 font-semibold text-foreground">Overview:</h3>
            <p className="whitespace-pre-line text-sm text-muted-foreground">{q.overview}</p>
          </section>
        )}
        <QuoteSections sections={q?.sections} placement={QuoteSectionPlacement.BEFORE_PRICING} />
        {renderPricing()}
        <QuoteSections sections={q?.sections} placement={QuoteSectionPlacement.AFTER_PRICING} />
        {renderSubscriptionDetails()}
        {renderContractTerms()}
        {q?.termsAndServices && (
          <section className="mb-8">
            <h3 className="mb-2 font-semibold text-foreground">Terms & Services:</h3>
            <p className="whitespace-pre-line text-sm text-muted-foreground">
              {q.termsAndServices}
            </p>
          </section>
        )}
        <Slot name="quoteView.attachments" attachments={q?.attachments ?? []} mode={mode} />
        {renderSignatures()}
      </CardContent>
    </Card>
  )
}

const months = (n: number) =>
  n % 12 === 0 ? `${n / 12} year${n === 12 ? '' : 's'}` : `${n} months`

/** The term the quote signs, as its subscription will renew (ADR-0022 §2). */
const ContractTermRows = ({ term }: { term: QuoteContractTerm }) => (
  <div className="grid gap-x-8 gap-y-2 lg:grid-cols-2">
    <DetailRow label="Term:" value={`${months(term.termMonths)} from the billing start`} />
    <DetailRow
      label="Renewal:"
      value={
        term.autoRenew
          ? `Renews automatically for ${months(term.renewalMonths)}${
              term.upliftPercent ? `, prices +${term.upliftPercent}% at each renewal` : ''
            }`
          : 'Ends with the term'
      }
    />
    {term.autoRenew && term.noticeDays > 0 && (
      <DetailRow
        label="Notice:"
        value={`Cancel at least ${term.noticeDays} days before a term ends`}
      />
    )}
  </div>
)
