import { Badge, Button } from '@md/ui'
import { Download, Eye } from 'lucide-react'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'
import { listInvoices } from '@/rpc/portal/customer/v1/customer-PortalCustomerService_connectquery'
import { InvoiceSummary } from '@/rpc/portal/customer/v1/models_pb'
import { formatCurrency } from '@/utils/numbers'
import { useSearchParams } from 'react-router-dom'

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
    return <div className="text-center py-8 text-sm text-muted-foreground">Loading invoices...</div>
  }

  if (invoices.length === 0) {
    return <div className="text-center py-8 text-sm text-muted-foreground">No invoices found</div>
  }

  return (
    <div className="space-y-3">
      {invoices.map(invoice => (
        <div
          key={invoice.id}
          className="flex items-center justify-between p-4 border border-gray-200 rounded-lg hover:border-gray-300 transition-colors"
        >
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="font-medium text-sm text-gray-900">{invoice.invoiceNumber}</span>
              <Badge variant={getInvoiceStatusVariant(invoice.status)} className="text-xs">
                {getInvoiceStatusLabel(invoice.status)}
              </Badge>
            </div>
            <div className="text-xs text-gray-600 mt-1">
              {invoice.invoiceDate} • {formatCurrency(Number(invoice.totalCents), invoice.currency)}
            </div>
          </div>
          <div className="flex items-center gap-1 ml-4">
            <Button
              size="sm"
              variant="ghost"
              onClick={() => handleViewInvoice(invoice.id)}
              title="View Invoice"
              className="h-8 w-8 p-0"
            >
              <Eye className="h-4 w-4" />
            </Button>
            {invoice.documentSharingKey && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => handleDownloadInvoice(invoice)}
                title="Download PDF"
                className="h-8 w-8 p-0"
              >
                <Download className="h-4 w-4" />
              </Button>
            )}
          </div>
        </div>
      ))}

      {/* Pagination */}
      {(page > 0 || hasMore) && (
        <div className="flex justify-between items-center pt-2">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setPage(p => Math.max(0, p - 1))}
            disabled={page === 0}
            className="text-xs"
          >
            Previous
          </Button>
          <span className="text-xs text-gray-600">Page {page + 1}</span>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setPage(p => p + 1)}
            disabled={!hasMore}
            className="text-xs"
          >
            Next
          </Button>
        </div>
      )}
    </div>
  )
}

const getInvoiceStatusLabel = (status: InvoiceStatus) => {
  const statusMap: Record<InvoiceStatus, string> = {
    [InvoiceStatus.DRAFT]: 'Draft',
    [InvoiceStatus.FINALIZED]: 'Finalized',
    [InvoiceStatus.VOID]: 'Void',
    [InvoiceStatus.UNCOLLECTIBLE]: 'Uncollectible',
  }
  return statusMap[status] || 'Unknown'
}

const getInvoiceStatusVariant = (
  status: InvoiceStatus
): 'default' | 'secondary' | 'destructive' => {
  if (status === InvoiceStatus.FINALIZED) return 'default'
  if (status === InvoiceStatus.VOID || status === InvoiceStatus.UNCOLLECTIBLE) return 'destructive'
  return 'secondary'
}
