import { Separator } from '@md/ui'
import { Flex } from '@ui/components/legacy'

import { parseAndFormatDate } from '@/utils/date'
import { formatCurrency } from '@/utils/numbers'

import { InvoicePaymentData } from '../types'

export const InvoiceSummary = ({ invoicePaymentData }: InvoicePaymentData) => {
  const { invoice, customer, tradeName, logoUrl } = invoicePaymentData

  if (!invoice) {
    return <div>Loading invoice...</div>
  }

  return (
    <div className="rounded-lg bg-white p-6 shadow-sm border">
      {/* Header with logo and company name */}
      <div className="mb-6">
        <div className="flex items-center gap-3 mb-4">
          {logoUrl && (
            <img src={logoUrl} alt={tradeName} className="h-8 w-auto" />
          )}
          <h2 className="text-lg font-semibold">{tradeName}</h2>
        </div>
        <h3 className="text-xl font-bold">Invoice {invoice.invoiceNumber}</h3>
      </div>

      {/* Customer Information */}
      <div className="mb-6">
        <h4 className="text-sm font-medium text-gray-700 mb-2">Bill To</h4>
        <div className="text-sm">
          <div className="font-medium">{customer?.name}</div>
          {customer?.billingAddress && (
            <div className="text-gray-600 mt-1">
              {customer.billingAddress.line1}
              {customer.billingAddress.line2 && (
                <>
                  <br />
                  {customer.billingAddress.line2}
                </>
              )}
              <br />
              {customer.billingAddress.city}, {customer.billingAddress.state} {customer.billingAddress.zipCode}
              <br />
              {customer.billingAddress.country}
            </div>
          )}
        </div>
      </div>

      <Separator className="my-4" />

      {/* Invoice Details */}
      <div className="space-y-3 mb-6">
        <Flex justify="space-between">
          <span className="text-sm text-gray-600">Invoice Date</span>
          <span className="text-sm font-medium">{parseAndFormatDate(invoice.invoiceDate)}</span>
        </Flex>
        
        {invoice.dueAt && (
          <Flex justify="space-between">
            <span className="text-sm text-gray-600">Due Date</span>
            <span className="text-sm font-medium">{parseAndFormatDate(invoice.dueAt)}</span>
          </Flex>
        )}
      </div>

      <Separator className="my-4" />

      {/* Line Items */}
      <div className="space-y-3 mb-6">
        <h4 className="text-sm font-medium text-gray-700">Items</h4>
        {invoice.lineItems?.map((item, index) => (
          <Flex key={item.id || index} justify="space-between" align="flex-start">
            <div className="flex-1">
              <div className="text-sm font-medium">{item.name}</div>
              {item.startDate && item.endDate && (
                <div className="text-xs text-gray-500 mt-1">
                  {parseAndFormatDate(item.startDate)} - {parseAndFormatDate(item.endDate)}
                </div>
              )}
            </div>
            <div className="text-sm font-medium">
              {formatCurrency(Number(item.subtotal) || 0, invoice.currency)}
            </div>
          </Flex>
        ))}
      </div>

      <Separator className="my-4" />

      {/* Totals */}
      <div className="space-y-2">
        <Flex justify="space-between">
          <span className="text-sm text-gray-600">Subtotal</span>
          <span className="text-sm">{formatCurrency(Number(invoice.subtotal) || 0, invoice.currency)}</span>
        </Flex>

        {invoice.taxAmount && Number(invoice.taxAmount) > 0 ? (
          <Flex justify="space-between">
            <span className="text-sm text-gray-600">Tax</span>
            <span className="text-sm">{formatCurrency(Number(invoice.taxAmount) || 0, invoice.currency)}</span>
          </Flex>
        ) : null}

        <Separator className="my-2" />

        <Flex justify="space-between">
          <span className="text-base font-semibold">Total</span>
          <span className="text-base font-semibold">
            {formatCurrency(Number(invoice.total) || 0, invoice.currency)}
          </span>
        </Flex>

        <Flex justify="space-between">
          <span className="text-lg font-bold text-brand">Amount Due</span>
          <span className="text-lg font-bold text-brand">
            {formatCurrency(Number(invoice.amountDue) || 0, invoice.currency)}
          </span>
        </Flex>
      </div>
    </div>
  )
}