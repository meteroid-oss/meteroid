import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { spaces } from '@md/foundation'
import {
  FormItem,
  Input,
  Modal,
  Form,
  FormInput,
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetFooter,
  Button,
  Separator,
} from '@ui2/components'
import { Flex } from '@ui2/components/legacy'
import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'

import ConfirmationModal from '@/components/ConfirmationModal'
import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'
import {
  createProduct,
  listProducts,
} from '@/rpc/api/products/v1/products-ProductsService_connectquery'

interface ProductEditPanelProps {
  visible: boolean
  closePanel: () => void
}
export const ProductEditPanel = ({ visible, closePanel }: ProductEditPanelProps) => {
  const [isClosingPanel, setIsClosingPanel] = useState(false)

  const queryClient = useQueryClient()

  const createProductMut = useMutation(createProduct, {
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: createConnectQueryKey(listProducts) })
    },
  })

  const methods = useZodForm({
    schema: schemas.products.createProductSchema,
    defaultValues: {
      name: '',
      description: '',
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

  // TODO try without the form, with onConfirm
  return (
    <>
      <Sheet key="TableEditor" open={visible} onOpenChange={safeClosePanel}>
        <SheetContent size={'small'}>
          <Form {...methods}>
            <form
              onSubmit={methods.handleSubmit(async values => {
                await createProductMut.mutateAsync(values)
                methods.reset()
                closePanel()
              })}
            >
              <SheetHeader className="pb-2">
                <SheetTitle>Create a new product item</SheetTitle>
                <Separator />
              </SheetHeader>
              <Flex direction="column" gap={spaces.space7}>
                <FormInput name="name" label="Product Name" type="text" placeholder="ACME Inc" />
                <FormInput name="description" label="Description" type="text" placeholder="desc" />
              </Flex>

              <SheetFooter className="py-2">
                <Button type="submit">Create</Button>
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
