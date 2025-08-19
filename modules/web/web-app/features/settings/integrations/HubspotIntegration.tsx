import { useMutation } from '@connectrpc/connect-query'
import {
  CheckboxFormField,
  DialogDescription,
  DialogTitle, Form,
  Modal,
} from '@md/ui'
import { UsersIcon } from 'lucide-react'
import { useNavigate } from 'react-router'
import { toast } from 'sonner'
import { z } from 'zod'

import { hubspotIntegrationSchema } from "@/features/settings/integrations/schemas";
import { useZodForm } from "@/hooks/useZodForm";
import {
  connectHubspot,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'


export const HubspotIntegrationModal = () => {
  const navigate = useNavigate()

  const methods = useZodForm({
    mode: 'onChange',
    schema: hubspotIntegrationSchema,
    defaultValues: {
      autoSync: true,
    },
  })

  const connectHubspotMutation = useMutation(connectHubspot, {
    onSuccess: (resp) => {
      window.location.href = resp.authUrl
    },
  })

  const onSubmit = async (data: z.infer<typeof hubspotIntegrationSchema>) => {
    try {
      await connectHubspotMutation.mutateAsync({
        data: {
          autoSync: data.autoSync,
        },
      })
    } catch (error) {
      toast.error(`Failed to connect. ${error instanceof Error ? error.message : 'Unknown error'}`)
    }
  }

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <UsersIcon className="w-6 h-6 text-blue"/>
            <span>Connect Hubspot</span>
          </DialogTitle>
          <DialogDescription className="text-sm">
            Connect your Hubspot account to synchronize your CRM data.
          </DialogDescription>
        </>
      }
      visible={true}
      hideFooter={false}
      onCancel={() => navigate('..')}
      onConfirm={methods.handleSubmit(onSubmit)}
    >
      <Modal.Content>
        <Form {...methods}>
          <form autoComplete="off">
            <div className="space-y-6">
              <CheckboxFormField
                label='Auto-sync new data between Meteroid and Hubspot'
                control={methods.control}
                name='autoSync'
              />
            </div>
          </form>
        </Form>
      </Modal.Content>
    </Modal>
  )
}
