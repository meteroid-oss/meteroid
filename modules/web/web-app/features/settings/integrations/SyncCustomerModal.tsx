import { useMutation } from '@connectrpc/connect-query'
import {
  DialogDescription,
  DialogTitle,
  Modal,
} from '@md/ui'
import { FolderSyncIcon } from 'lucide-react'
import { toast } from 'sonner'

import { syncToHubspot, syncToPennylane } from "@/rpc/api/customers/v1/customers-CustomersService_connectquery";

export const enum IntegrationType {
  Hubspot = 'Hubspot',
  Pennylane = 'Pennylane',
}

interface CustomerProps {
  id: string
  name: string
  integrationType: IntegrationType
  onClose: () => void
}

export const SyncCustomerModal = ({ id, name, integrationType, onClose }: CustomerProps) => {
  const syncToHubspotMutation = useMutation(syncToHubspot, {
    onSuccess: () => {
      toast.success('Sync request sent!')
    },
  })

  const syncToPennylaneMutation = useMutation(syncToPennylane, {
    onSuccess: () => {
      toast.success('Sync request sent!')
    },
  })

  const onConfirm = async () => {
    try {
      if (integrationType === IntegrationType.Pennylane) {
        await syncToPennylaneMutation.mutateAsync({
          customerIds: [id],
        })
      } else if (integrationType === IntegrationType.Hubspot) {
        await syncToHubspotMutation.mutateAsync({
          customerIds: [id],
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
            <span>{name}: Sync customer data to {integrationType}</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Send a request to sync customer data to {integrationType}. It might take a few minutes
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
