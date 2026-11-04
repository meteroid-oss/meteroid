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
      <button
        type="button"
        onClick={handleDownload}
        className="inline-flex items-center text-sm font-medium hover:opacity-80 transition-opacity"
        style={{ color: 'var(--mtp-text-2)' }}
      >
        <Download className="h-4 w-4 mr-2" />
        Download PDF
      </button>
    )
  }

  return (
    <div
      className="mt-6 p-4"
      style={{
        border: '1px solid var(--mtp-border)',
        borderRadius: 'var(--mtp-r-ctrl)',
        background: 'var(--mtp-surface)',
      }}
    >
      <div className="flex items-start justify-between">
        <div className="flex items-start">
          <FileText className="h-5 w-5 mt-0.5" style={{ color: 'var(--mtp-text-2)' }} />
          <div className="ml-3">
            <h4 className="text-sm font-medium" style={{ color: 'var(--mtp-text)' }}>
              Invoice Document
            </h4>
            <p className="mt-1 text-sm" style={{ color: 'var(--mtp-text-2)' }}>
              {invoiceNumber ? `Invoice ${invoiceNumber}` : 'Invoice PDF'}
            </p>
          </div>
        </div>
        <button
          type="button"
          onClick={handleDownload}
          className="inline-flex items-center shrink-0 text-sm font-medium px-3 py-1.5 transition-opacity hover:opacity-80"
          style={{
            border: '1px solid var(--mtp-border-2)',
            borderRadius: 'var(--mtp-r-sm)',
            color: 'var(--mtp-text)',
            background: 'var(--mtp-surface-2)',
          }}
        >
          <Download className="h-4 w-4 mr-2" />
          Download
        </button>
      </div>
    </div>
  )
}
