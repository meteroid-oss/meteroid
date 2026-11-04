import { useMutation } from '@connectrpc/connect-query'
import { DialogDescription, DialogTitle, Modal } from '@md/ui'
import { FolderSyncIcon } from 'lucide-react'
import { toast } from 'sonner'

import { syncCreditNoteToPennylane } from '@/rpc/api/creditnotes/v1/creditnotes-CreditNotesService_connectquery'

import { IntegrationType } from './SyncInvoiceModal'

interface CreditNoteProps {
  id: string
  creditNoteNumber: string
  integrationType: IntegrationType
  onClose: () => void
}

export const SyncCreditNoteModal = ({
  id,
  creditNoteNumber,
  integrationType,
  onClose,
}: CreditNoteProps) => {
  const syncMutation = useMutation(syncCreditNoteToPennylane, {
    onSuccess: () => {
      toast.success('Sync request sent!')
    },
  })

  const onConfirm = async () => {
    try {
      if (integrationType === IntegrationType.Pennylane) {
        await syncMutation.mutateAsync({ creditNoteIds: [id] })
      }
    } catch (error) {
      toast.error(
        `Failed to send sync request: ${error instanceof Error ? error.message : 'Unknown error'}`
      )
    }
    onClose()
  }

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <FolderSyncIcon className="w-6 h-6 text-blue" />
            <span>
              {creditNoteNumber}: Sync credit note data to {integrationType}
            </span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Send a request to sync credit note data to {integrationType}. It might take a few
            minutes to get processed.
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={false}
      onCancel={() => onClose()}
      onConfirm={onConfirm}
    ></Modal>
  )
}
