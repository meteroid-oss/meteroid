import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  CheckboxFormField, DialogDescription,
  DialogTitle, Form,
  Modal,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Edit2Icon } from 'lucide-react'
import { useEffect } from "react";
import { useNavigate, useParams } from 'react-router'
import { toast } from 'sonner'
import { z } from 'zod'

import { Loading } from "@/components/Loading";
import { hubspotIntegrationSchema } from "@/features/settings/integrations/schemas";
import { useZodForm } from "@/hooks/useZodForm";
import { useQuery } from "@/lib/connectrpc";
import {
  listConnectors,
  updateHubspotConnector,
} from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'


export const EditHubspotIntegrationModal = () => {
  const navigate = useNavigate()

  const { connectionId } = useParams();

  const connectorsQuery = useQuery(listConnectors, {})

  const methods = useZodForm({
    mode: 'onChange',
    schema: hubspotIntegrationSchema,
    defaultValues: {
      autoSync: false
    },
  })

  const connection = connectorsQuery.data?.connectors.find(conn => conn.id === connectionId)

  useEffect(() => {
    if (connection?.data?.data?.case == 'hubspot') {
      methods.reset({
        autoSync: connection.data.data.value.autoSync,
      })
    }
  }, [connection])

  const queryClient = useQueryClient()
  const updateHubspotConnectorMutation = useMutation(updateHubspotConnector, {
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey(listConnectors),
      })
      toast.success('Connection updated!')
    },
    onError: (error) => {
      toast.error(`Connection update failure. ${error instanceof Error ? error.message : 'Unknown error'}`)
    },
  })

  const onSubmit = async (data: z.infer<typeof hubspotIntegrationSchema>) => {
    await updateHubspotConnectorMutation.mutateAsync({
      id: connectionId,
      autoSync: data.autoSync,
    })
    navigate('..')
  }

  if (!connection) {
    return <Loading/>
  }

  return (
    <Modal
      header={
        <>
          <DialogTitle className="flex items-center gap-2 text-md">
            <Edit2Icon className="w-6 h-6 text-blue"/>
            <span>Edit Hubspot Integration</span>
          </DialogTitle>
          <DialogDescription/>
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
