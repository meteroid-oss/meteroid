import { useMutation } from '@connectrpc/connect-query'
import {
  DialogDescription,
  DialogTitle,
  Modal,
} from '@md/ui'
import { BanknoteIcon } from 'lucide-react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'

import {
  connectPennylane,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'

export const PennylaneIntegrationModal = () => {
  const navigate = useNavigate()

  const connectPennylaneMutation = useMutation(connectPennylane, {
    onSuccess: (resp) => {
      console.log(resp.authUrl)
      window.location.href = resp.authUrl
    },
  })

  const onConfirm = async () => {
    try {
      await connectPennylaneMutation.mutateAsync({
        data: {},
      })
    } catch (error) {
      toast.error(`Failed to connect: ${error instanceof Error ? error.message : 'Unknown error'}`)
    }
  }

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <BanknoteIcon className="w-6 h-6 text-blue"/>
            <span>Connect Pennylane</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Connect your Pennylane account to synchronize your financial data. <br/>
            You will be redirected to Pennylane to sign-in and authorize
            the connection.
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={false}
      onCancel={() => navigate('..')}
      onConfirm={onConfirm}
    >
    </Modal>
  )
}
