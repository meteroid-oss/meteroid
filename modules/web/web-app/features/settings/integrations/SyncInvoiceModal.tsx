import { useMutation } from '@connectrpc/connect-query'
import {
  DialogDescription,
  DialogTitle,
  Modal,
} from '@md/ui'
import { FolderSyncIcon } from 'lucide-react'
import { toast } from 'sonner'

import { syncToPennylane } from "@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery";

export const enum IntegrationType {
  Pennylane = 'Pennylane',
}

interface InvoiceProps {
  id: string
  invoiceNumber: string
  integrationType: IntegrationType
  onClose: () => void
}

export const SyncInvoiceModal = ({ id, invoiceNumber, integrationType, onClose }: InvoiceProps) => {
  const syncToPennylaneMutation = useMutation(syncToPennylane, {
    onSuccess: () => {
      toast.success('Sync request sent!')
    },
  })

  const onConfirm = async () => {
    try {
      if (integrationType === IntegrationType.Pennylane) {
        await syncToPennylaneMutation.mutateAsync({
          invoiceIds: [id],
        })
      }
    } catch (error) {
      toast.error(`Failed to send sync request: ${error instanceof Error ? error.message : 'Unknown error'}`)
    }
    onClose()
  }

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <FolderSyncIcon className="w-6 h-6 text-blue"/>
            <span>{invoiceNumber}: Sync invoice data to {integrationType}</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Send a request to sync invoice data to {integrationType}. It might take a few minutes
            to get processed.
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
