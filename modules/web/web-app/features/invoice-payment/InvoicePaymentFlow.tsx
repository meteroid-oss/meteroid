import { useMutation } from '@connectrpc/connect-query'
import { ArrowLeft } from 'lucide-react'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'

import { PaymentPanel } from '@/features/checkout/PaymentPanel'
import { BillingInfo } from '@/features/checkout/components/BillingInfo'
import { ReadonlyPaymentView } from '@/features/checkout/components/ReadonlyPaymentView'
import { getInvoicePaymentAvailability } from '@/features/checkout/utils/paymentAvailability'
import { confirmInvoicePayment } from '@/rpc/portal/invoice/v1/invoice-PortalInvoiceService_connectquery'
import { formatCurrency } from '@/utils/numbers'

import { BankTransferInfo } from './components/BankTransferInfo'
import { InvoicePdfDownload } from './components/InvoicePdfDownload'
import { InvoiceSummary } from './components/InvoiceSummary'
import { TransactionList } from './components/TransactionList'
import { InvoicePaymentData } from './types'

/**
 * Main invoice payment flow component
 */
const InvoicePaymentFlow: React.FC<InvoicePaymentData> = ({ invoicePaymentData }) => {
  const [isAddressEditing, setIsAddressEditing] = useState(false)
  const navigate = useNavigate()
  const {
    invoice,
    customer,
    paymentMethods,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
  } = invoicePaymentData

  // Mutation to confirm the invoice payment
  const confirmInvoicePaymentMutation = useMutation(confirmInvoicePayment, {
    onError: error => {
      console.error('Invoice payment confirmation error:', error)
    },
  })

  /**
   * Process payment with selected payment method
   */
  const handlePaymentSubmit = async (paymentMethodId: string) => {
    try {
      if (!invoice?.currency) {
        throw new Error('Currency is not defined')
      }

      await confirmInvoicePaymentMutation.mutateAsync({
        displayedAmount: invoice.amountDue,
        displayedCurrency: invoice.currency,
        paymentMethodId,
      })

      // On success, redirect to success page (we can create this later)
      const params = new URLSearchParams({
        invoice: invoice.invoiceNumber || '',
        customer: customer?.name || '',
      })
      navigate(`success?${params.toString()}`)
    } catch (error) {
      console.error('Payment submission error:', error)
      throw error // Let the PaymentPanel handle this error
    }
  }

  if (!invoice || !customer) {
    return <div className="p-8 text-center">Loading invoice payment information...</div>
  }

  // Determine what payment UI to show
  const paymentAvailability = getInvoicePaymentAvailability({
    invoiceStatus: invoice.status,
    paymentStatus: invoice.paymentStatus,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
    hasTransactions: (invoice.transactions?.length ?? 0) > 0,
  })

  return (
    <div className="flex flex-col lg:flex-row min-h-screen">
      {/* Mobile header */}
      <div className="lg:hidden w-full p-4 border-b border-gray-100 flex items-center">
        <button className=" flex items-center" onClick={() => window.history.back()}>
          <ArrowLeft size={16} className="mr-2" />
          <span className="mr-2">{invoicePaymentData.tradeName}</span>
          {invoicePaymentData.logoUrl && (
            <img src={invoicePaymentData.logoUrl} alt="logo" width={24} height={24} />
          )}
        </button>
        <div className="text-sm font-medium mx-auto">Pay Invoice {invoice.invoiceNumber}</div>
      </div>

      {/* Main content */}
      <div className="min-h-screen max-h-screen w-full flex md:flex-row flex-col overflow-auto">
        {/* Left panel - Invoice summary */}
        <div className="flex flex-col md:h-screen bg-background-gray gap-5 px-5 md:px-4 lg:px-20 lg:pt-16 lg:pb-20 pt-5 pb-5 border-b border-border-regular md:pb-8 md:pt-16 w-full md:overflow-auto">
          <div className="md:max-w-[500px] w-full ml-auto  ">
            <InvoiceSummary invoicePaymentData={invoicePaymentData} />
          </div>
        </div>
        {/* Right panel - Payment form */}
        <div className="w-full flex lg:px-20 md:px-4 px-5 flex-col bg-white md:h-screen md:overflow-auto lg:pt-16 py-5 shadow-md">
          <div className="mr-auto ml-auto md:ml-0 md:pt-0 md:h-screen w-full max-w-[440px]">
            {/* Billing information */}
            <BillingInfo
              customer={customer}
              isEditing={isAddressEditing}
              setIsEditing={setIsAddressEditing}
            />

            {/* Invoice PDF Download */}
            <InvoicePdfDownload
              invoiceId={invoice.id}
              invoiceNumber={invoice.invoiceNumber}
              documentSharingKey={invoice.documentSharingKey}
              pdfDocumentId={invoice.pdfDocumentId}
            />

            {/* Render based on payment availability */}
            {paymentAvailability.type === 'readonly' && (
              <>
                <ReadonlyPaymentView
                  reason={paymentAvailability.reason}
                  displayTransactions={paymentAvailability.displayTransactions}
                />
                {paymentAvailability.displayTransactions && invoice.transactions && (
                  <TransactionList transactions={invoice.transactions} currency={invoice.currency} />
                )}
              </>
            )}

            {paymentAvailability.type === 'bank_only' && (
              <BankTransferInfo
                bankAccount={paymentAvailability.bankAccount}
                invoiceNumber={invoice.invoiceNumber}
                customerName={customer?.name}
              />
            )}

            {paymentAvailability.type === 'payment_form' && (
              <>
                {/* Show payment panel if card or DD available */}
                {(paymentAvailability.cardConnectionId ||
                  paymentAvailability.directDebitConnectionId) && (
                  <PaymentPanel
                    customer={customer}
                    paymentMethods={paymentMethods || []}
                    currency={invoice.currency}
                    totalAmount={formatCurrency(Number(invoice.amountDue) || 0, invoice.currency)}
                    onPaymentSubmit={handlePaymentSubmit}
                    cardConnectionId={paymentAvailability.cardConnectionId}
                    directDebitConnectionId={paymentAvailability.directDebitConnectionId}
                  />
                )}

                {/* Show bank transfer as alternative if available */}
                {paymentAvailability.bankAccount &&
                  (paymentAvailability.cardConnectionId ||
                    paymentAvailability.directDebitConnectionId) && (
                    <div className="mt-6">
                      <div className="text-center text-sm text-gray-500 mb-4">or</div>
                      <BankTransferInfo
                        bankAccount={paymentAvailability.bankAccount}
                        invoiceNumber={invoice.invoiceNumber}
                        customerName={customer?.name}
                      />
                    </div>
                  )}
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

export default InvoicePaymentFlow
