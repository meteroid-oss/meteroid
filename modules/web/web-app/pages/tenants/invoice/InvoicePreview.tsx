import { Dialog, DialogContent } from '@md/ui'

import { useQuery } from '@/lib/connectrpc'
import { previewInvoiceHtml } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { useEffect } from 'react'

export const PreviewInvoiceDialog = ({
  open,
  setOpen,
  invoiceId,
}: {
  open: boolean
  setOpen: (open: boolean) => void
  invoiceId: string
}) => {
  const previewInvoiceHtmlQuery = useQuery(previewInvoiceHtml, { id: invoiceId }, { gcTime: 0 })

  useEffect(() => {
    if (open) {
      previewInvoiceHtmlQuery.refetch()
    }
  }, [open])

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="w-full min-h-[870px]  max-w-[824px] p-2 bg-muted">
        {previewInvoiceHtmlQuery.isLoading ? (
          <>Loading</>
        ) : (
          <iframe
            srcDoc={previewInvoiceHtmlQuery.data?.html}
            className="w-full h-full border border-border rounded-sm bg-white"
          ></iframe>
        )}
      </DialogContent>
    </Dialog>
  )
}
