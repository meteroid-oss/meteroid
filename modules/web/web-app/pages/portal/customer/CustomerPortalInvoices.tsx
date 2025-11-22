import { Badge } from '@md/ui'
import { Download } from 'lucide-react'
import { useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { InvoicePaymentStatus } from '@/rpc/api/invoices/v1/models_pb'
import { listInvoices } from '@/rpc/portal/customer/v1/customer-PortalCustomerService_connectquery'
import { InvoiceSummary } from '@/rpc/portal/customer/v1/models_pb'
import { formatCurrency } from '@/utils/numbers'

export const CustomerPortalInvoices = () => {
  const [page, setPage] = useState(0)
  const pageSize = 5

  const invoicesQuery = useQuery(listInvoices, {
    pagination: {
      perPage: pageSize,
      page: page,
    },
  })

  const [searchParams] = useSearchParams()

  const token = searchParams.get('token')

  const invoices = invoicesQuery.data?.invoices || []
  const hasMore = invoices.length === pageSize

  const handleViewInvoice = (invoiceId: string) => {
    window.open(`/portal/invoice-payment/${invoiceId}?token=${token}`, '_blank')
  }

  const handleDownloadInvoice = (invoice: InvoiceSummary) => {
    if (invoice.documentSharingKey && invoice.id) {
      const downloadUrl = `${env.meteroidRestApiUri}/files/v1/invoice/pdf/${invoice.id}?token=${invoice.documentSharingKey}`
      window.open(downloadUrl, '_blank')
    }
  }

  if (invoicesQuery.isLoading) {
    return (
      <div className="text-center py-6">
        <div className="inline-flex items-center gap-2 text-xs text-gray-500">
          <svg className="animate-spin h-4 w-4" viewBox="0 0 24 24">
            <circle
              className="opacity-25"
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              strokeWidth="4"
              fill="none"
            />
            <path
              className="opacity-75"
              fill="currentColor"
              d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
            />
          </svg>
          Loading...
        </div>
      </div>
    )
  }

  if (invoices.length === 0) {
    return (
      <div className="text-center py-4">
        <p className="text-xs text-gray-500">No invoices yet</p>
      </div>
    )
  }

  return (
    <div>
      <div className="space-y-0">
        {invoices.map((invoice, index) => (
          <div
            key={invoice.id}
            className={`flex items-center justify-between py-2 text-sm ${
              index !== invoices.length - 1 ? 'border-b border-gray-100' : ''
            }`}
          >
            <div className="flex items-center gap-4 flex-1">
              <div className="w-24 text-gray-900">{invoice.invoiceDate}</div>
              <div className="flex items-center gap-2">
                <span className="font-medium text-gray-900">
                  {formatCurrency(Number(invoice.totalCents), invoice.currency)}
                </span>
                <Badge
                  variant={getInvoicePaymentStatusVariant(invoice.paymentStatus)}
                  className="text-xs"
                >
                  {getInvoicePaymentStatusLabel(invoice.paymentStatus)}
                </Badge>
              </div>
              {invoice.planName && <div className="text-gray-500 text-xs">{invoice.planName}</div>}
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={() => handleViewInvoice(invoice.id)}
                className="text-xs text-gray-600 hover:text-gray-900 font-medium"
              >
                View
              </button>
              {invoice.documentSharingKey && (
                <button
                  onClick={() => handleDownloadInvoice(invoice)}
                  className="p-1.5 hover:bg-gray-100 rounded transition-colors"
                  title="Download PDF"
                >
                  <Download className="h-3.5 w-3.5 text-gray-600" />
                </button>
              )}
            </div>
          </div>
        ))}
      </div>

      {/* Pagination */}
      {(page > 0 || hasMore) && (
        <div className="flex justify-between items-center pt-3 mt-2 border-t border-gray-100">
          <button
            onClick={() => setPage(p => Math.max(0, p - 1))}
            disabled={page === 0}
            className="text-xs text-gray-600 hover:text-gray-900 font-medium disabled:opacity-40 disabled:cursor-not-allowed"
          >
            ← Previous
          </button>
          <span className="text-xs text-gray-500">Page {page + 1}</span>
          <button
            onClick={() => setPage(p => p + 1)}
            disabled={!hasMore}
            className="text-xs text-gray-600 hover:text-gray-900 font-medium disabled:opacity-40 disabled:cursor-not-allowed"
          >
            Next →
          </button>
        </div>
      )}
    </div>
  )
}

const getInvoicePaymentStatusLabel = (status: InvoicePaymentStatus) => {
  const statusMap: Record<InvoicePaymentStatus, string> = {
    [InvoicePaymentStatus.ERRORED]: 'Errored',
    [InvoicePaymentStatus.PAID]: 'Paid',
    [InvoicePaymentStatus.PARTIALLY_PAID]: 'Partially Paid',
    [InvoicePaymentStatus.UNPAID]: 'Unpaid',
  }
  return statusMap[status] || 'Unknown'
}

const getInvoicePaymentStatusVariant = (
  status: InvoicePaymentStatus
): 'default' | 'secondary' | 'destructive' | 'success' | 'warning' => {
  if (status === InvoicePaymentStatus.PAID) return 'success'
  if (status === InvoicePaymentStatus.ERRORED) return 'destructive'
  return 'warning'
}
