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
  Separator,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Flex } from '@ui/components/legacy'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'

import ConfirmationModal from '@/components/ConfirmationModal'
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
      <Sheet open={visible} onOpenChange={safeClosePanel}>
        <SheetContent size="medium">
          <Form {...methods}>
            <form
              onSubmit={methods.handleSubmit(async values => {
                const res = await createCustomerMut.mutateAsync({
                  data: {
                    name: values.companyName,
                    alias: values.alias,
                  },
                })
                if (res.customer?.id) {
                  navigate(`./${res.customer.id}`)
                }
              })}
            >
              <SheetHeader className="border-b border-border pb-3">
                <SheetTitle>Create a new customer</SheetTitle>
              </SheetHeader>
              <div className="py-6">
                <Flex direction="column" gap={spaces.space7}>
                  <h2 className="text-lg font-semibold text-muted-foreground">
                    Customer Information
                  </h2>

                  <FormField
                    control={methods.control}
                    name="companyName"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Customer Name</FormLabel>
                        <FormControl>
                          <Input type="text" placeholder="ACME Inc" {...field} autoComplete="off" />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={methods.control}
                    name="primaryEmail"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Primary email</FormLabel>
                        <FormControl>
                          <Input type="text" {...field} autoComplete="off" />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={methods.control}
                    name="alias"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Alias (external id)</FormLabel>
                        <FormControl>
                          <Input type="text" {...field} autoComplete="off" />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <Separator />

                  <h2 className="text-lg font-semibold text-muted-foreground">Invoicing Method</h2>

                  <SheetDescription>
                    In this release, the only billing method available is via <b>Stripe Invoice</b>
                  </SheetDescription>

                  <FormField
                    control={methods.control}
                    name="stripeCustomerId"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Stripe Customer ID</FormLabel>
                        <FormControl>
                          <Input type="text" {...field} />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </Flex>
              </div>
              <SheetFooter>
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
