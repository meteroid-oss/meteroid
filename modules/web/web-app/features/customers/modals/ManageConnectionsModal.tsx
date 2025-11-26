import { useMutation, useQuery } from '@connectrpc/connect-query'
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Separator,
} from '@ui/components'
import { Trash2 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import {
  deleteCustomerConnection,
  upsertCustomerConnection,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { Customer } from '@/rpc/api/customers/v1/models_pb'

interface ManageConnectionsModalProps {
  openState: [boolean, React.Dispatch<React.SetStateAction<boolean>>]
  customer: Customer | undefined
  onSuccess?: () => void
}

interface ConnectionFormData {
  connectorId: string
  externalCustomerId: string
  existingConnectionId?: string
}

export const ManageConnectionsModal = ({
  openState: [visible, setVisible],
  customer,
  onSuccess,
}: ManageConnectionsModalProps) => {
  const [formData, setFormData] = useState<Record<string, ConnectionFormData>>({})

  const connectorsQuery = useQuery(listConnectors, {})

  const upsertMutation = useMutation(upsertCustomerConnection, {
    onSuccess: () => {
      toast.success('Connection saved successfully')
      onSuccess?.()
    },
    onError: error => {
      toast.error(`Failed to save connection: ${error.message}`)
    },
  })

  const deleteMutation = useMutation(deleteCustomerConnection, {
    onSuccess: () => {
      toast.success('Connection deleted successfully')
      onSuccess?.()
    },
    onError: error => {
      toast.error(`Failed to delete connection: ${error.message}`)
    },
  })

  // Initialize form data when customer or connectors change
  useEffect(() => {
    if (!customer || !connectorsQuery.data?.connectors) return

    const newFormData: Record<string, ConnectionFormData> = {}

    connectorsQuery.data.connectors.forEach(connector => {
      const existingConnection = customer.customerConnections?.find(
        conn => conn.connectorId === connector.id
      )

      newFormData[connector.id] = {
        connectorId: connector.id,
        externalCustomerId: existingConnection?.externalCustomerId || '',
        existingConnectionId: existingConnection?.id,
      }
    })

    setFormData(newFormData)
  }, [customer, connectorsQuery.data])

  const getProviderName = (provider: ConnectorProviderEnum): string => {
    switch (provider) {
      case ConnectorProviderEnum.STRIPE:
        return 'Stripe'
      case ConnectorProviderEnum.HUBSPOT:
        return 'Hubspot'
      case ConnectorProviderEnum.PENNYLANE:
        return 'Pennylane'
      default:
        return 'Unknown'
    }
  }

  const handleSave = async (connectorId: string) => {
    if (!customer) return

    const data = formData[connectorId]
    if (!data || !data.externalCustomerId.trim()) {
      toast.error('External customer ID is required')
      return
    }

    await upsertMutation.mutateAsync({
      customerId: customer.id,
      connectorId: data.connectorId,
      externalCustomerId: data.externalCustomerId.trim(),
    })
  }

  const handleDelete = async (connectorId: string) => {
    if (!customer) return

    const data = formData[connectorId]
    if (!data?.existingConnectionId) return

    await deleteMutation.mutateAsync({
      customerId: customer.id,
      connectionId: data.existingConnectionId,
    })

    // Clear the form field after deletion
    setFormData(prev => ({
      ...prev,
      [connectorId]: {
        ...prev[connectorId],
        externalCustomerId: '',
        existingConnectionId: undefined,
      },
    }))
  }

  const handleInputChange = (connectorId: string, value: string) => {
    setFormData(prev => ({
      ...prev,
      [connectorId]: {
        ...prev[connectorId],
        externalCustomerId: value,
      },
    }))
  }

  const isPending = upsertMutation.isPending || deleteMutation.isPending

  return (
    <Dialog open={visible} onOpenChange={setVisible}>
      <DialogContent className="max-w-2xl max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Manage External Connections</DialogTitle>
          <DialogDescription>
            Manually connect this customer to external providers by entering their external customer
            IDs.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 pb-2">
          {connectorsQuery.isLoading && (
            <div className="text-sm text-muted-foreground">Loading connectors...</div>
          )}

          {connectorsQuery.data?.connectors.map(connector => {
            const data = formData[connector.id]
            const hasExistingConnection = !!data?.existingConnectionId

            return (
              <div key={connector.id} className="space-y-2 p-2 ">
                <Separator />
                <div className="flex items-center justify-between">
                  <div>
                    <Label className="text-sm font-medium">
                      {getProviderName(connector.provider)}
                    </Label>
                    <div className="text-xs text-muted-foreground">{connector.alias}</div>
                  </div>
                  {hasExistingConnection && (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => handleDelete(connector.id)}
                      disabled={isPending}
                    >
                      <Trash2 className="h-4 w-4" />
                    </Button>
                  )}
                </div>

                <div className="flex gap-2">
                  <div className="flex-1">
                    <Input
                      placeholder="External customer ID"
                      value={data?.externalCustomerId || ''}
                      onChange={e => handleInputChange(connector.id, e.target.value)}
                      disabled={isPending}
                    />
                  </div>
                  <Button
                    size="sm"
                    onClick={() => handleSave(connector.id)}
                    disabled={isPending || !data?.externalCustomerId?.trim()}
                  >
                    {hasExistingConnection ? 'Update' : 'Add'}
                  </Button>
                </div>
              </div>
            )
          })}
        </div>

        <DialogFooter>
          <Button size="sm" variant="secondary" onClick={() => setVisible(false)}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
