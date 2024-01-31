import { useMutation, createConnectQueryKey } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import { Flex, FormItem, Input, Modal, SidePanel } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { HelpCircleIcon } from 'lucide-react'
import { useState } from 'react'


import ConfirmationModal from '@/components/atoms/ConfirmationModal'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import {
  createCustomer,
  listCustomers,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { CustomerBillingConfig_Stripe_CollectionMethod } from '@/rpc/api/customers/v1/models_pb'

interface CustomersEditPanelProps {
  visible: boolean
  closePanel: () => void
}
export const CustomersEditPanel = ({ visible, closePanel }: CustomersEditPanelProps) => {
  const [isClosingPanel, setIsClosingPanel] = useState(false)

  const queryClient = useQueryClient()

  const createCustomerMut = useMutation(createCustomer, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listCustomers) })
    },
  })
  const methods = useZodForm({
    schema: schemas.customers.createCustomerSchema,
  })

  const safeClosePanel = () => {
    const isDirty = methods.formState.isDirty
    if (isDirty) {
      setIsClosingPanel(true)
    } else {
      methods.reset()
      closePanel()
    }
  }

  // TODO try without the form, with onConfirm
  return (
    <>
      <SidePanel
        size="large"
        key="TableEditor"
        visible={visible}
        header={<SidePanel.HeaderTitle>Create a new customer</SidePanel.HeaderTitle>}
        className={`transition-all duration-100 ease-in `}
        onCancel={safeClosePanel}
        onConfirm={methods.handleSubmit(async values => {
          await createCustomerMut.mutateAsync({
            name: values.companyName,
            alias: values.externalId,
            billingConfig: {
              billingConfigOneof: {
                case: 'stripe',
                value: {
                  collectionMethod:
                    CustomerBillingConfig_Stripe_CollectionMethod.CHARGE_AUTOMATICALLY, // TODO
                  customerId: values.stripeCustomerId,
                },
              },
            },
          })
          methods.reset()
          closePanel()
        })}
        onInteractOutside={event => {
          const isToast = (event.target as Element)?.closest('#toast')
          if (isToast) {
            event.preventDefault()
          }
        }}
      >
        <SidePanel.Content>
          <Flex direction="column" gap={spaces.space7}>
            <FormItem name="name" label="Customer Name" {...methods.withError('companyName')}>
              <Input type="text" placeholder="ACME Inc" {...methods.register('companyName')} />
            </FormItem>

            <FormItem name="name" label="Primary email" {...methods.withError('primaryEmail')}>
              <Input type="text" {...methods.register('primaryEmail')} />
            </FormItem>

            <FormItem name="name" label="External ID" optional {...methods.withError('externalId')}>
              <Input type="text" {...methods.register('externalId')} />
            </FormItem>

            <span className="border p-3 text-xs text-slate-900 border-slate-600 rounded-md">
              <HelpCircleIcon size={14} className="inline-block mr-2" />
              In this release, the only invoicing method available is Stripe Invoice
            </span>
            <FormItem
              name="name"
              label="Stripe Customer ID"
              {...methods.withError('stripeCustomerId')}
            >
              <Input type="text" {...methods.register('stripeCustomerId')} />
            </FormItem>
          </Flex>
        </SidePanel.Content>
      </SidePanel>
      <ConfirmationModal
        visible={isClosingPanel}
        header="Confirm to close"
        buttonLabel="Confirm"
        onSelectCancel={() => setIsClosingPanel(false)}
        onSelectConfirm={() => {
          setIsClosingPanel(false)
          methods.reset()
          closePanel()
        }}
      >
        <Modal.Content>
          <p className="py-4 text-sm text-scale-1100">
            There are unsaved changes. Are you sure you want to close the panel? Your changes will
            be lost.
          </p>
        </Modal.Content>
      </ConfirmationModal>
    </>
  )
}
