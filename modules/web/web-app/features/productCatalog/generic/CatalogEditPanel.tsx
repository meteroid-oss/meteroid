import { spaces } from '@md/foundation'
import {
  Button,
  Form,
  Modal,
  Separator,
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@md/ui'
import { Flex } from '@ui/components/legacy'
import { useState } from 'react'
import { z } from 'zod'

import ConfirmationModal from '@/components/ConfirmationModal'
import { Methods } from '@/hooks/useZodForm'

export interface CatalogEditPanelProps<T extends z.ZodTypeAny> {
  title: string
  visible: boolean
  methods: Methods<T>
  closePanel: () => void
  onSubmit: (values: z.infer<T>) => Promise<void>
  children?: React.ReactNode
}
export function CatalogEditPanel<T extends z.ZodTypeAny>({
  visible,
  closePanel,
  title,
  methods,
  onSubmit,
  children,
}: CatalogEditPanelProps<T>) {
  const [isClosingPanel, setIsClosingPanel] = useState(false)

  // TODO useConfirmationModal

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
      <Sheet key="TableEditor" open={visible} onOpenChange={safeClosePanel}>
        <SheetContent size="small">
          <Form {...methods}>
            <form
              /* @ts-expect-error react hook form generic breaks */
              onSubmit={methods.handleSubmit(async (values) => {
                await onSubmit(values)
                methods.reset()
                closePanel()
              })}
            >
              <SheetHeader className="pb-2">
                <SheetTitle>{title}</SheetTitle>
                <Separator />
              </SheetHeader>
              <Flex direction="column" gap={spaces.space7}>
                {children}
              </Flex>

              <SheetFooter className="py-2">
                <Button type="submit">Save</Button>
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
