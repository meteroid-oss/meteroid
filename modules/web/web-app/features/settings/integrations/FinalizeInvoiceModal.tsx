import { useMutation } from '@connectrpc/connect-query'
import {
  DialogDescription,
  DialogTitle,
  Modal,
} from '@md/ui'
import { CheckCircleIcon } from 'lucide-react'
import { toast } from 'sonner'

import { finalizeInvoice } from "@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery";

interface FinalizeInvoiceModalProps {
  id: string
  invoiceNumber: string
  onClose: () => void
  onSuccess?: () => void
}

export const FinalizeInvoiceModal = ({ id, invoiceNumber, onClose, onSuccess }: FinalizeInvoiceModalProps) => {
  const finalizeInvoiceMutation = useMutation(finalizeInvoice, {
    onSuccess: () => {
      toast.success('Invoice finalized successfully!')
      onSuccess?.()
    },
  })

  const onConfirm = async () => {
    try {
      await finalizeInvoiceMutation.mutateAsync({
        id: id,
      })
    } catch (error) {
      toast.error(`Failed to finalize invoice: ${error instanceof Error ? error.message : 'Unknown error'}`)
    }
    onClose()
  }

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <CheckCircleIcon className="w-6 h-6 text-green-600"/>
            <span>{invoiceNumber}: Finalize & Send Invoice</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Finalize this invoice and send it to the customer. Once finalized, the invoice cannot be edited.
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={false}
      onCancel={() => onClose()}
      onConfirm={onConfirm}
    >
    </Modal>
  )
}
