import { Separator } from '@md/ui'

import { PaymentMethodBadge } from '@/features/invoice-payment/components/TransactionList'
import { InvoicePaymentStatus } from '@/rpc/api/invoices/v1/models_pb'
import { parseAndFormatDate } from '@/utils/date'
import { formatCurrency, formatCurrencyNoRounding } from '@/utils/numbers'

import { InvoicePaymentData } from '../types'

export const InvoiceSummary = ({ invoicePaymentData }: InvoicePaymentData) => {
  const {
    invoice,
    customer,
    tradeName,
    logoUrl,
    bankAccount,
    footerLegal,
    legalNumber,
    footerInfo,
  } = invoicePaymentData

  if (!invoice) {
    return <div>Loading invoice...</div>
  }

  const getPaymentStatusLabel = (status: InvoicePaymentStatus) => {
    switch (status) {
      case InvoicePaymentStatus.PAID:
        return 'Paid'
      case InvoicePaymentStatus.PARTIALLY_PAID:
        return 'Partially Paid'
      case InvoicePaymentStatus.UNPAID:
        return 'Unpaid'
      case InvoicePaymentStatus.ERRORED:
        return 'Errored'
      default:
        return 'Unpaid'
    }
  }

  const getPaymentStatusColor = (status: InvoicePaymentStatus) => {
    switch (status) {
      case InvoicePaymentStatus.PAID:
        return 'bg-green-50 text-green-700'
      case InvoicePaymentStatus.PARTIALLY_PAID:
        return 'bg-orange-50 text-orange-700'
      case InvoicePaymentStatus.UNPAID:
        return 'bg-red-50 text-red-700'
      case InvoicePaymentStatus.ERRORED:
        return 'bg-red-50 text-red-700'
      default:
        return 'bg-gray-50 text-gray-700'
    }
  }

  const customerDetails = invoice.customerDetails

  // Net terms calculation for payment terms
  const paymentTermsText =
    invoice.netTerms > 0
      ? `Payment is due within ${invoice.netTerms} days from the invoice date.`
      : 'Payment is due upon receipt.'

  return (
    <div className="flex flex-col rounded-lg bg-white p-9 shadow-sm border max-w-[850px] md:min-h-[850px]  mx-auto text-sm text-gray-900">
      {/* Header section with Invoice title, info grid, and logo */}
      <div className="grid grid-cols-12 gap-6 mb-10">
        {/* Left: Invoice title and info grid */}
        <div className="col-span-9">
          <h1 className="text-2xl font-medium text-gray-900 mb-4">Invoice</h1>

          <div className="grid grid-cols-[120px_auto] gap-y-1.5">
            <span className="text-gray-600">Invoice number</span>
            <span>{invoice.invoiceNumber}</span>

            <span className="text-gray-600">Issue date</span>
            <span>{parseAndFormatDate(invoice.invoiceDate)}</span>

            {invoice.dueAt && (
              <>
                <span className="text-gray-600">Due date</span>
                <span>{parseAndFormatDate(invoice.dueAt)}</span>
              </>
            )}

            {invoice.purchaseOrder && (
              <>
                <span className="text-gray-600">Purchase order</span>
                <span>{invoice.purchaseOrder}</span>
              </>
            )}

            {customerDetails?.vatNumber && (
              <>
                <span className="text-gray-600">VAT ID</span>
                <span>{customerDetails.vatNumber}</span>
              </>
            )}
          </div>
        </div>

        {/* Right: Logo */}
        <div className="col-span-3 flex justify-end">
          {logoUrl && <img src={logoUrl} alt={tradeName} className="h-9 w-auto object-contain" />}
        </div>
      </div>

      {/* Company and client info with amount due section */}
      <div className="grid grid-cols-12 gap-6 mb-8">
        {/* From (Organization) */}
        <div className="col-span-3">
          <div className="text-gray-900 mb-1.5">{tradeName}</div>
        </div>

        {/* Bill To */}
        <div className="col-span-3">
          <div className="text-gray-900 mb-1.5">Bill to</div>
          <div className="text-gray-600">
            <div>{customerDetails?.name || customer?.name}</div>
            {customerDetails?.billingAddress && (
              <>
                {customerDetails.billingAddress.line1 && (
                  <div>{customerDetails.billingAddress.line1}</div>
                )}
                {customerDetails.billingAddress.line2 && (
                  <div>{customerDetails.billingAddress.line2}</div>
                )}
                {(customerDetails.billingAddress.zipCode ||
                  customerDetails.billingAddress.city) && (
                  <div>
                    {customerDetails.billingAddress.zipCode} {customerDetails.billingAddress.city}
                  </div>
                )}
                {customerDetails.billingAddress.country && (
                  <div>{customerDetails.billingAddress.country}</div>
                )}
              </>
            )}
            {customerDetails?.email && <div>{customerDetails.email}</div>}
          </div>
        </div>

        {/* Amount Due Section */}
        <div className="col-span-6 text-right">
          <div className="text-2xl font-medium text-gray-900">
            {formatCurrency(invoice.total, invoice.currency)}
          </div>
          {invoice.dueAt && (
            <div className="text-base text-gray-900 mt-0.5">
              due {parseAndFormatDate(invoice.dueAt)}
            </div>
          )}
          {invoice.memo && <div className="text-gray-600 mt-2">{invoice.memo}</div>}
        </div>
      </div>

      {/* Line Items Table */}
      <div className="mb-4">
        {/* Table Header */}
        <div className="grid grid-cols-12 gap-2 pb-2 border-b border-gray-300 text-xs text-gray-600">
          <div className="col-span-5">Description</div>
          <div className="col-span-2 text-center">Quantity</div>
          <div className="col-span-2 text-right">Unit price</div>
          <div className="col-span-1 text-right">Tax rate</div>
          <div className="col-span-2 text-right">Amount</div>
        </div>

        {/* Table Body */}
        <div className="divide-y divide-gray-100">
          {invoice.lineItems?.map((item, index) => (
            <div key={item.id || index} className="py-3">
              <div className="grid grid-cols-12 gap-2">
                <div className="col-span-5">
                  <div className="text-gray-900">{item.name}</div>
                  {item.description && (
                    <div className="text-xs text-gray-600 mt-0.5">{item.description}</div>
                  )}
                  {item.startDate && item.endDate && (
                    <div className="text-xs text-gray-500 mt-0.5">
                      {parseAndFormatDate(item.startDate)} → {parseAndFormatDate(item.endDate)}
                    </div>
                  )}
                </div>
                <div className="col-span-2 text-center text-gray-900">{item.quantity || ''}</div>
                <div className="col-span-2 text-right text-gray-900">
                  {item.unitPrice ? formatCurrencyNoRounding(item.unitPrice, invoice.currency) : ''}
                </div>
                <div className="col-span-1 text-right text-gray-900">
                  {item.taxRate ? `${item.taxRate}%` : ''}
                </div>
                <div className="col-span-2 text-right text-gray-900">
                  {formatCurrency(item.subtotal, invoice.currency)}
                </div>
              </div>

              {/* Sub-line items */}
              {item.subLineItems && item.subLineItems.length > 0 && (
                <div className="mt-1.5 ml-3 space-y-1">
                  {item.subLineItems.map((subItem, subIndex) => (
                    <div
                      key={subItem.id || subIndex}
                      className="grid grid-cols-12 gap-2 text-xs text-gray-600"
                    >
                      <div className="col-span-5 pl-3">{subItem.name}</div>
                      <div className="col-span-2 text-center">{subItem.quantity || ''}</div>
                      <div className="col-span-2 text-right">
                        {subItem.unitPrice
                          ? formatCurrencyNoRounding(subItem.unitPrice, invoice.currency)
                          : ''}
                      </div>
                      <div className="col-span-1"></div>
                      <div className="col-span-2 text-right">
                        {formatCurrency(subItem.total, invoice.currency)}
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      </div>

      {/* Summary section with payment status and totals */}
      <div className="grid grid-cols-2 gap-10 mb-8">
        {/* Left: Payment Status (only shown if not unpaid) */}
        <div>
          {invoice.paymentStatus !== InvoicePaymentStatus.UNPAID && (
            <>
              <Separator className="mb-3" />
              <div className="flex items-center justify-between mb-3">
                <span className="text-gray-900">Payment status</span>
                <span
                  className={`px-2 py-1 rounded text-xs ${getPaymentStatusColor(invoice.paymentStatus)}`}
                >
                  {getPaymentStatusLabel(invoice.paymentStatus)}
                </span>
              </div>

              {/* Transactions list */}
              {invoice.transactions && invoice.transactions.length > 0 && (
                <div className="mt-2">
                  <div className="hidden grid-cols-3 gap-4 text-xs text-gray-600 mb-2 ">
                    <div>Payment method</div>
                    <div>Payment date</div>
                    <div>Payment amount</div>
                  </div>
                  {invoice.transactions.map(transaction => (
                    <div key={transaction.id} className="grid grid-cols-3 gap-4 mb-2">
                      <div>
                        <PaymentMethodBadge paymentMethodInfo={transaction.paymentMethodInfo} />
                      </div>
                      <div>
                        {transaction.processedAt
                          ? parseAndFormatDate(transaction.processedAt)
                          : 'N/A'}
                      </div>
                      <div>{formatCurrency(transaction.amount, invoice.currency)}</div>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </div>

        {/* Right: Totals */}
        <div>
          <Separator className="mb-3" />

          <div className="space-y-2">
            <>
              <div className="flex justify-between">
                <span className="text-gray-600">Subtotal</span>
                <span>{formatCurrency(invoice.subtotal, invoice.currency)}</span>
              </div>

              {invoice.couponLineItems && invoice.couponLineItems.length > 0
                ? invoice.couponLineItems.map(coupon => (
                    <div key={coupon.couponId} className="flex justify-between">
                      <span className="text-gray-600">{coupon.name}</span>
                      <span>-{formatCurrency(coupon.total, invoice.currency)}</span>
                    </div>
                  ))
                : invoice.discount &&
                  Number(invoice.discount) > 0 && (
                    <div className="flex justify-between">
                      <span className="text-gray-600">Discount</span>
                      <span>-{formatCurrency(invoice.discount, invoice.currency)}</span>
                    </div>
                  )}

              {invoice.taxBreakdown &&
              invoice.taxBreakdown.length > 0 &&
              Number(invoice.taxAmount) > 0
                ? invoice.taxBreakdown.map((taxItem, index) => (
                    <div key={index} className="flex justify-between">
                      <span className="text-gray-600">
                        {taxItem.name} {taxItem.taxRate}%
                      </span>
                      <span>{formatCurrency(taxItem.amount, invoice.currency)}</span>
                    </div>
                  ))
                : null}

              <Separator className="my-2" />

              <div className="flex justify-between text-base">
                <span className="text-gray-900">Total due</span>
                <span className="text-gray-900">
                  {formatCurrency(invoice.total, invoice.currency)}
                </span>
              </div>
            </>
          </div>
        </div>
      </div>

      {/* Payment Information Section */}
      {bankAccount?.data && (
        <>
          <Separator className="mb-4" />
          <div className="mb-6">
            <div className="text-gray-900 mb-2">Payment information</div>
            <div className="grid grid-cols-[160px_auto] gap-y-2 gap-x-3">
              {bankAccount.data.bankName && (
                <>
                  <span className="text-gray-900">Bank name</span>
                  <span className="text-gray-600">{bankAccount.data.bankName}</span>
                </>
              )}
              {bankAccount.data.country && (
                <>
                  <span className="text-gray-900">Country</span>
                  <span className="text-gray-600">{bankAccount.data.country}</span>
                </>
              )}
              {bankAccount.data.format?.case === 'ibanBicSwift' && (
                <>
                  <span className="text-gray-900">IBAN</span>
                  <span className="text-gray-600 font-mono">
                    {bankAccount.data.format.value.iban}
                  </span>
                  {bankAccount.data.format.value.bicSwift && (
                    <>
                      <span className="text-gray-900">BIC/SWIFT</span>
                      <span className="text-gray-600 font-mono">
                        {bankAccount.data.format.value.bicSwift}
                      </span>
                    </>
                  )}
                </>
              )}
              {bankAccount.data.format?.case === 'accountNumberBicSwift' && (
                <>
                  <span className="text-gray-900">Account number</span>
                  <span className="text-gray-600 font-mono">
                    {bankAccount.data.format.value.accountNumber}
                  </span>
                  <span className="text-gray-900">BIC/SWIFT</span>
                  <span className="text-gray-600 font-mono">
                    {bankAccount.data.format.value.bicSwift}
                  </span>
                </>
              )}
              {bankAccount.data.format?.case === 'accountNumberRoutingNumber' && (
                <>
                  <span className="text-gray-900">Account number</span>
                  <span className="text-gray-600 font-mono">
                    {bankAccount.data.format.value.accountNumber}
                  </span>
                  <span className="text-gray-900">Routing number</span>
                  <span className="text-gray-600 font-mono">
                    {bankAccount.data.format.value.routingNumber}
                  </span>
                </>
              )}
              {bankAccount.data.format?.case === 'sortCodeAccountNumber' && (
                <>
                  <span className="text-gray-900">Sort code</span>
                  <span className="text-gray-600 font-mono">
                    {bankAccount.data.format.value.sortCode}
                  </span>
                  <span className="text-gray-900">Account number</span>
                  <span className="text-gray-600 font-mono">
                    {bankAccount.data.format.value.accountNumber}
                  </span>
                </>
              )}
            </div>
          </div>
        </>
      )}

      {/* Payment Terms and Tax Info Section */}
      <Separator className="mb-4" />
      <div className="grid grid-cols-2 gap-10 mb-6">
        {/* Payment Terms */}
        <div>
          <div className="text-gray-900 mb-2">Payment terms</div>
          <div className="text-gray-600 text-xs">{paymentTermsText}</div>
        </div>

        {/* Tax Info */}
        <div>
          <div className="text-gray-900 mb-2">Tax information</div>
          <div className="text-gray-600 text-xs">
            {invoice.taxBreakdown && invoice.taxBreakdown.length > 0 ? (
              <>
                {invoice.taxBreakdown.some(item => item.name.toLowerCase().includes('reverse')) ? (
                  <div>Reverse charge mechanism applies. VAT is payable by the recipient.</div>
                ) : invoice.taxBreakdown.some(item =>
                    item.name.toLowerCase().includes('exempt')
                  ) ? (
                  <div>This invoice is exempt from VAT.</div>
                ) : (
                  <div>All prices include applicable taxes.</div>
                )}
              </>
            ) : (
              <div>No tax applied</div>
            )}
          </div>
        </div>
      </div>

      {/* Legal Information */}
      {footerLegal && (
        <div className="mb-6">
          <div className="text-gray-900 mb-2">Legal information</div>
          <div className="text-gray-500 text-xs whitespace-pre-line">{footerLegal}</div>
          {legalNumber && (
            <div className="text-gray-500 text-xs mt-1">Company registration: {legalNumber}</div>
          )}
        </div>
      )}

      {/* Footer Custom Information */}
      {footerInfo && (
        <div className="mb-6">
          <div className="text-gray-500 text-xs whitespace-pre-line">{footerInfo}</div>
        </div>
      )}

      <div className="my-auto"></div>

      {/* Footer with branding and invoice summary */}
      <div className="pt-12 mt-6 align-bottom">
        <div className="text-xs text-gray-500 flex items-center gap-1.5 flex-wrap">
          <a
            href="https://meteroid.com?utm_source=invoice"
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center hover:opacity-80"
          >
            <img src="/img/meteroid-logo-wordmark--light.svg" alt="meteroid" className="h-3" />
          </a>
          <span>•</span>
          <a
            href="https://meteroid.com?utm_source=invoice"
            target="_blank"
            rel="noopener noreferrer"
            className="text-gray-900 hover:underline"
          >
            Billing automation for SaaS
          </a>
          <span>•</span>
          <span>
            {invoice.invoiceNumber} • {formatCurrency(invoice.total, invoice.currency)} due{' '}
            {invoice.dueAt && parseAndFormatDate(invoice.dueAt)}
          </span>
        </div>
      </div>
    </div>
  )
}
