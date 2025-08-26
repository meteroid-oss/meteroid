import { useMutation } from '@connectrpc/connect-query'
import {
  DialogDescription,
  DialogTitle,
  Modal,
} from '@md/ui'
import { FolderSyncIcon } from 'lucide-react'
import { toast } from 'sonner'

import { syncToHubspot } from "@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery";

export const enum IntegrationType {
  Hubspot = 'Hubspot',
}

interface SubscriptionProps {
  id: string
  customerName: string
  integrationType: IntegrationType
  onClose: () => void
}

export const SyncSubscriptionModal = ({ id, customerName, integrationType, onClose }: SubscriptionProps) => {
  const syncToHubspotMutation = useMutation(syncToHubspot, {
    onSuccess: () => {
      toast.success('Sync request sent!')
    },
  })

  const onConfirm = async () => {
    try {
      if (integrationType === IntegrationType.Hubspot) {
        await syncToHubspotMutation.mutateAsync({
          subscriptionIds: [id],
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
            <span>{customerName}: Sync subscription data to {integrationType}</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Send a request to sync subscription data to {integrationType}. It might take a few minutes
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
