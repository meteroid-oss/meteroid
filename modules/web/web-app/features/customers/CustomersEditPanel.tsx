import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import {
  Button,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  Input,
  Modal,
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Flex } from '@ui/components/legacy'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'

import ConfirmationModal from '@/components/ConfirmationModal'
import { CustomersBilling } from '@/features/customers/form/CustomersBilling'
import { CustomersGeneral } from '@/features/customers/form/CustomersGeneral'
import { CustomersInvoice } from '@/features/customers/form/CustomersInvoice'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import {
  createCustomer,
  listCustomers,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'

interface CustomersEditPanelProps {
  visible: boolean
  closePanel: () => void
}

export const CustomersEditPanel = ({ visible, closePanel }: CustomersEditPanelProps) => {
  const [isClosingPanel, setIsClosingPanel] = useState(false)

  const queryClient = useQueryClient()

  const navigate = useNavigate()

  const createCustomerMut = useMutation(createCustomer, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listCustomers) })
    },
  })

  const methods = useZodForm({
    schema: schemas.customers.createCustomerSchema,
    defaultValues: {
      paymentTerm: 30,
      gracePeriod: 7,
      taxRate: 20,
      shipping: false,
    },
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

  return (
    <>
      <Sheet open={visible} onOpenChange={safeClosePanel}>
        <SheetContent size="medium" side="right" className="p-0">
          <Form {...methods}>
            <form
              onSubmit={methods.handleSubmit(async values => {
                const res = await createCustomerMut.mutateAsync({
                  data: {
                    name: values.companyName,
                    alias: values.alias,
                    // Add more fields to the API call as needed
                    // You'll need to update your API to accept all these new fields
                  },
                })
                if (res.customer?.id) {
                  navigate(`./${res.customer.id}`)
                }
              })}
              className="flex h-full w-full flex-col"
            >
              <SheetHeader className="ml-6 mt-4">
                <SheetTitle>Create customer</SheetTitle>
              </SheetHeader>

              <div className="flex-1 overflow-y-auto">
                <div className="space-y-8 p-6">
                  <CustomersGeneral />
                  <CustomersBilling />
                  <CustomersInvoice />

                  {/* Integrations Section */}
                  <Flex direction="column" gap={spaces.space4}>
                    <h2 className="font-medium">Integrations</h2>

                    <FormField
                      control={methods.control}
                      name="connectorCustomerId"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>Connector Customer ID</FormLabel>
                          <FormControl>
                            <Input
                              type="text"
                              placeholder="Integration ID"
                              {...field}
                              autoComplete="off"
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </Flex>
                </div>
              </div>

              <SheetFooter className="border-t border-border p-3">
                <Button variant="outline">Cancel</Button>
                <Button type="submit">Save changes</Button>
              </SheetFooter>
            </form>
          </Form>
        </SheetContent>
      </Sheet>

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
          <p className="py-4 text-sm text-muted-foreground">
            There are unsaved changes. Are you sure you want to close the panel? Your changes will
            be lost.
          </p>
        </Modal.Content>
      </ConfirmationModal>
    </>
  )
}
