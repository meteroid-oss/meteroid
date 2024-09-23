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
  const getCountriesQuery = useQuery(previewInvoiceHtml, { id: invoiceId }, { gcTime: 0 })

  useEffect(() => {
    if (open) {
      getCountriesQuery.refetch()
    }
  }, [open])

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="w-full h-[80vh]  max-w-screen-lg">
        {getCountriesQuery.isLoading ? (
          <>Loading</>
        ) : (
          <iframe srcDoc={getCountriesQuery.data?.html} className="w-full h-full"></iframe>
        )}
      </DialogContent>
    </Dialog>
  )
}
