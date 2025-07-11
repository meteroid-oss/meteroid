import { useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Form,
  Modal,
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useNavigate } from 'react-router-dom'

import ConfirmationModal from '@/components/ConfirmationModal'
import { Loading } from '@/components/Loading'
import { CustomersGeneral } from '@/features/customers/form/CustomersGeneral'
import { useZodForm } from '@/hooks/useZodForm'
import { useQuery } from '@/lib/connectrpc'
import { schemas } from '@/lib/schemas'
import {
  createCustomer,
  listCustomers,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

interface CustomersEditPanelProps {
  visible: boolean
  closePanel: () => void
}

export const CustomersEditPanel = ({ visible, closePanel }: CustomersEditPanelProps) => {
  const queryClient = useQueryClient()

  const navigate = useNavigate()

  // const [integrationsVisible, setIntegrationsVisible] = useState(false)
  const [isClosingPanel, setIsClosingPanel] = useState(false)

  const createCustomerMut = useMutation(createCustomer, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: [listCustomers.service.typeName] })
    },
  })

  const activeCurrenciesQuery = useQuery(listTenantCurrencies)
  const activeCurrencies = activeCurrenciesQuery.data?.currencies ?? []

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
              onSubmitCapture={() => console.log(methods.formState.errors)}
              onSubmit={methods.handleSubmit(async values => {
                const res = await createCustomerMut.mutateAsync({
                  data: {
                    name: values.companyName,
                    alias: values.alias,
                    billingEmail: values.primaryEmail,
                    currency: values.currency,
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
                  {activeCurrenciesQuery.isLoading ? (
                    <Loading />
                  ) : (
                    <CustomersGeneral activeCurrencies={activeCurrencies} />
                  )}

                  {/* <CustomersBilling />
                  <CustomersInvoice /> */}

                  {/* Integrations Section */}
                  {/* <Flex direction="column" className="gap-2">
                    <Flex
                      align="center"
                      className="gap-2 cursor-pointer group"
                      onClick={() => setIntegrationsVisible(!integrationsVisible)}
                    >
                      <h2 className="font-medium">Integrations</h2>
                      <ChevronRight
                        size={14}
                        className={`text-muted-foreground transition-transform duration-200 ease-in-out ${
                          integrationsVisible ? 'rotate-90' : ''
                        }`}
                      />
                    </Flex>
                    {integrationsVisible && (
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
                    )}
                  </Flex> */}
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
