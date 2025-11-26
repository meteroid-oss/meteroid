import { Button } from '@md/ui'
import { Download, FileText } from 'lucide-react'

import { env } from '@/lib/env'

interface InvoicePdfDownloadProps {
  invoiceId: string
  invoiceNumber?: string | null
  documentSharingKey?: string | null
  pdfDocumentId?: string | null
  compact?: boolean
}

export const InvoicePdfDownload: React.FC<InvoicePdfDownloadProps> = ({
  invoiceId,
  invoiceNumber,
  documentSharingKey,
  pdfDocumentId,
  compact = false,
}) => {
  if (!documentSharingKey || !pdfDocumentId) {
    return null
  }

  const handleDownload = () => {
    const pdfUrl = `${env.meteroidRestApiUri}/files/v1/invoice/pdf/${invoiceId}?token=${documentSharingKey}`
    window.open(pdfUrl, '_blank')
  }

  if (compact) {
    return (
      <Button variant="ghost" size="sm" onClick={handleDownload} className="text-sm">
        <Download className="h-4 w-4 mr-2" />
        Download PDF
      </Button>
    )
  }

  return (
    <div className="mt-6 border border-gray-200 rounded-lg p-4">
      <div className="flex items-start justify-between">
        <div className="flex items-start">
          <FileText className="h-5 w-5 text-gray-600 mt-0.5" />
          <div className="ml-3">
            <h4 className="text-sm font-medium text-gray-900">Invoice Document</h4>
            <p className="mt-1 text-sm text-gray-600">
              {invoiceNumber ? `Invoice ${invoiceNumber}` : 'Invoice PDF'}
            </p>
          </div>
        </div>
        <Button variant="outline" size="sm" onClick={handleDownload}>
          <Download className="h-4 w-4 mr-2" />
          Download
        </Button>
      </div>
    </div>
  )
}
