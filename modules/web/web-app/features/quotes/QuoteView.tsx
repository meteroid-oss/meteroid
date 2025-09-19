import { PlainMessage } from '@bufbuild/protobuf'
import { Card, CardContent } from '@md/ui'
import { FC } from 'react'

import { AddressLinesCompact } from '@/features/customers/cards/address/AddressCard'
import {
  PricingComponent,
  SubscriptionPricingTable,
} from '@/features/subscriptions/pricecomponents/SubscriptionPricingTable'
import { env } from '@/lib/env'
import { Customer } from '@/rpc/api/customers/v1/models_pb'
import { InvoicingEntityPublic } from '@/rpc/api/invoicingentities/v1/models_pb'
import { Quote, QuoteComponent } from '@/rpc/api/quotes/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'

export interface QuoteViewProps {
  quote: {
    quote?: PlainMessage<Quote>
    invoicingEntity?: PlainMessage<InvoicingEntityPublic>
    customer?: PlainMessage<Customer>
    components?: PlainMessage<QuoteComponent>[]
  }
  mode?: 'preview' | 'detailed' | 'portal'
  className?: string
  subscriptionComponents?: PricingComponent[]
}

export const QuoteView: FC<QuoteViewProps> = ({
  quote,
  mode = 'detailed',
  className = '',
  subscriptionComponents,
}) => {
  const renderQuoteHeader = () => (
    <div className="mb-8">
      <div className="flex justify-between items-start mb-4">
        <div>
          <h1 className="text-2xl font-bold">Quote</h1>
          <p className="text-lg font-medium text-foreground">
            {quote.quote?.quoteNumber || 'DRAFT'}
          </p>
          <p className="text-sm text-muted-foreground mt-1">
            {quote.quote?.createdAt
              ? parseAndFormatDate(quote.quote.createdAt)
              : new Date().toLocaleDateString()}
          </p>
          {quote.quote?.expiresAt && (
            <p className="text-sm text-destructive mt-1">
              Expires: {parseAndFormatDate(quote.quote.expiresAt)}
            </p>
          )}
        </div>
        {/* Invoicing Entity Logo */}
        {quote.invoicingEntity?.logoAttachmentId && (
          <div className="w-16 h-16 rounded-lg overflow-hidden border bg-muted flex-shrink-0">
            <img
              src={
                env.meteroidRestApiUri + '/files/v1/logo/' + quote.invoicingEntity.logoAttachmentId
              }
              alt="Company logo"
              className="w-full h-full object-cover"
            />
          </div>
        )}
      </div>
    </div>
  )

  const renderFromToSection = () => (
    <div className="grid grid-cols-2 gap-8 mb-8">
      <div>
        <h3 className="font-semibold text-foreground mb-2">From:</h3>
        <div className="text-sm text-muted-foreground">
          {quote.invoicingEntity ? (
            <>
              <p className="font-medium text-foreground">{quote.invoicingEntity.legalName}</p>
              {quote.invoicingEntity.city && (
                <AddressLinesCompact
                  address={{
                    city: quote.invoicingEntity.city,
                    country: quote.invoicingEntity.country,
                    line1: quote.invoicingEntity.addressLine1,
                    line2: quote.invoicingEntity.addressLine2,
                    state: quote.invoicingEntity.state,
                  }}
                />
              )}
            </>
          ) : (
            <p className="font-medium text-foreground">Your Company</p>
          )}
        </div>
      </div>
      <div>
        <h3 className="font-semibold text-foreground mb-2">To:</h3>
        <div className="text-sm text-muted-foreground">
          {quote.customer ? (
            <>
              <p className="font-medium text-foreground">{quote.customer.name}</p>
              <p>{quote.customer.billingEmail}</p>
              {quote.customer.billingAddress && (
                <AddressLinesCompact address={quote.customer.billingAddress}/>
              )}
            </>
          ) : quote.quote?.customerId ? (
            <p className="font-medium text-foreground">Customer ID: {quote.quote.customerId}</p>
          ) : (
            <p className="font-medium  text-muted-foreground">No customer selected</p>
          )}
        </div>
      </div>
    </div>
  )

  const renderSubscriptionComponents = () => {
    // Use passed subscription components for preview mode, or convert quote components for other modes
    let pricingComponents: PricingComponent[]

    if (mode === 'preview' && subscriptionComponents) {
      pricingComponents = subscriptionComponents
    } else {
      // Convert QuoteComponents to PricingComponents for the shared table
      pricingComponents = quote.components || []
    }

    if (!quote.quote?.currency) {
      return (
        <div className="mb-8">
          <p className="font-medium  text-muted-foreground">No currency selected</p>
        </div>
      )
    }

    return (
      <div className="mb-8">
        <SubscriptionPricingTable
          components={pricingComponents}
          currency={quote.quote.currency}
          labelClassName="px-0"
        />
      </div>
    )
  }

  const renderSubscriptionDetails = () => {
    if (!quote.quote) return null

    return (
      <div className="mb-8">
        <h3 className="font-semibold text-foreground mb-4">Subscription Details</h3>
        <div className="bg-muted/50 p-4 rounded-lg space-y-2 grid lg:grid-cols-2">
          <div className="">
            <div className="flex justify-between">
              <span className="text-sm text-muted-foreground">Start Date:</span>
              <span className="text-sm">
                {quote.quote.startDate ? parseAndFormatDate(quote.quote.startDate) : 'Not set'}
              </span>
            </div>
            {quote.quote.endDate && (
              <div className="flex justify-between">
                <span className="text-sm text-muted-foreground">End Date:</span>
                <span className="text-sm">{parseAndFormatDate(quote.quote.endDate)}</span>
              </div>
            )}
            {quote.quote.billingStartDate && (
              <div className="flex justify-between">
                <span className="text-sm text-muted-foreground">Billing Start Date:</span>
                <span className="text-sm">{parseAndFormatDate(quote.quote.billingStartDate)}</span>
              </div>
            )}
            {quote.quote.trialDuration && quote.quote.trialDuration > 0 && (
              <div className="flex justify-between">
                <span className="text-sm text-muted-foreground">Trial Duration:</span>
                <span className="text-sm">{quote.quote.trialDuration} days</span>
              </div>
            )}
            {quote.quote.billingDayAnchor && (
              <div className="flex justify-between">
                <span className="text-sm text-muted-foreground">Billing Day:</span>
                <span className="text-sm">Day {quote.quote.billingDayAnchor} of each month</span>
              </div>
            )}

            {quote.quote.netTerms && (
              <div className="flex justify-between">
                <span className="text-sm text-muted-foreground">Payment Terms:</span>
                <span className="text-sm">Net {quote.quote.netTerms} days</span>
              </div>
            )}
          </div>
        </div>
      </div>
    )
  }

  const renderOverview = () => {
    if (!quote.quote?.overview) return null

    return (
      <div className="mb-8">
        <h3 className="font-semibold text-foreground mb-2">Overview:</h3>
        <p className="text-sm text-muted-foreground whitespace-pre-line">{quote.quote.overview}</p>
      </div>
    )
  }

  const renderAdditionalInfo = () => {
    const hasInfo = quote.quote?.termsAndServices || quote.quote?.internalNotes

    if (!hasInfo) return null

    return (
      <>
        {quote.quote?.termsAndServices && (
          <div className="mb-8">
            <h3 className="font-semibold text-foreground mb-2">Terms & Services:</h3>
            <p className="text-sm text-muted-foreground whitespace-pre-line">
              {quote.quote.termsAndServices}
            </p>
          </div>
        )}
      </>
    )
  }

  const renderSignatureArea = () => {
    if (mode === 'portal') return null

    return (
      <div className="border-t pt-8">
        <h3 className="font-semibold mb-4">Signature</h3>
        <div className="bg-muted/30 border-2 border-dashed border-muted-foreground/30 p-8 rounded-lg text-center">
          <p className="text-muted-foreground">Digital signatures</p>
        </div>
      </div>
    )
  }

  return (
    <Card className={className}>
      <CardContent className="p-8">
        {renderQuoteHeader()}
        {renderFromToSection()}
        {renderOverview()}
        {renderSubscriptionComponents()}
        {renderSubscriptionDetails()}
        {renderAdditionalInfo()}
        {renderSignatureArea()}
      </CardContent>
    </Card>
  )
}
