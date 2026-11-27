import * as Dialog from '@radix-ui/react-dialog'
import { VariantProps, cva } from 'class-variance-authority'
import { useEffect, useState } from 'react'

import { cn } from '@ui/lib'

import { Button } from './button'
import { Spinner } from './spinner'

const variants = cva(
  // Full width below `sm` (minus a 16px margin each side), so the surface can be a size container.
  'mt-modal mt-modal-surface relative my-4 w-[calc(100vw-2rem)] sm:w-full',
  {
    variants: {
      size: {
        tiny: `sm:align-middle sm:w-full sm:max-w-xs`,
        small: `sm:align-middle sm:w-full sm:max-w-sm`,
        medium: `sm:align-middle sm:w-full sm:max-w-lg`,
        large: `sm:align-middle sm:w-full max-w-xl`,
        xlarge: `sm:align-middle sm:w-full max-w-3xl`,
        xxlarge: `sm:align-middle sm:w-full max-w-6xl`,
        xxxlarge: `sm:align-middle sm:w-full max-w-7xl`,
      },
    },
    defaultVariants: {
      size: 'medium',
    },
  }
)

// import { Transition } from '@tailwindui/react'
// Merge Radix Props to surface in the modal component
export type ModalProps = RadixProps & Props

interface RadixProps
  extends Dialog.DialogProps,
    Pick<
      Dialog.DialogContentProps,
      | 'onOpenAutoFocus'
      | 'onCloseAutoFocus'
      | 'onEscapeKeyDown'
      | 'onPointerDownOutside'
      | 'onInteractOutside'
    > {}

interface Props {
  children?: React.ReactNode
  customFooter?: React.ReactNode
  hideFooter?: boolean
  alignFooter?: 'right' | 'left'
  layout?: 'horizontal' | 'vertical'
  loading?: boolean
  onCancel?: () => void
  cancelText?: string
  onConfirm?: () => void
  confirmText?: string
  footerBackground?: boolean
  visible: boolean
  className?: string
  triggerElement?: React.ReactNode
  header?: React.ReactNode
  confirmDisabled?: boolean
}

const Modal = ({
  children,
  customFooter = undefined,
  hideFooter = false,
  loading = false,
  cancelText = 'Cancel',
  onConfirm = () => {},
  onCancel = () => {},
  confirmText = 'Confirm',
  visible = false,
  className = '',
  triggerElement,
  header,
  size,
  confirmDisabled,
  ...props
}: ModalProps & VariantProps<typeof variants>) => {
  const [open, setOpen] = useState(visible ? visible : false)

  useEffect(() => {
    setOpen(visible)
  }, [visible])

  const footerContent = customFooter ? (
    customFooter
  ) : (
    <div className="mt-modal-footer flex w-full gap-2 justify-end">
      <Button variant="secondary" onClick={onCancel} disabled={loading} size="sm">
        {cancelText}
      </Button>
      <Button onClick={onConfirm} disabled={loading || confirmDisabled} hasIcon={loading} size="sm">
        {loading && <Spinner />}
        {confirmText}
      </Button>
    </div>
  )

  function handleOpenChange(open: boolean) {
    if (visible !== undefined && !open) {
      // controlled component behaviour
      onCancel()
    } else {
      // un-controlled component behaviour
      setOpen(open)
    }
  }

  return (
    <Dialog.Root open={open} onOpenChange={handleOpenChange}>
      {triggerElement && <Dialog.Trigger>{triggerElement}</Dialog.Trigger>}
      <Dialog.Portal>
        <Dialog.Overlay
          className="z-50
          fixed
          mt-backdrop
          h-full w-full
          left-0
          top-0
          mt-overlay"
        />
        <Dialog.Overlay
          className="z-50
          fixed
          inset-0
          grid
          place-items-center
          overflow-y-auto
          mt-modal-holder"
        >
          <Dialog.Content
            className={cn(variants({ size }), className)}
            onInteractOutside={props.onInteractOutside}
            onEscapeKeyDown={props.onEscapeKeyDown}
            aria-describedby={undefined}
          >
            {header && (
              <div className="px-4 pt-4 pb-3">
                <Dialog.Title className="font-display text-[15px] font-medium leading-none text-foreground">
                  {header}
                </Dialog.Title>
              </div>
            )}
            {!header && <Dialog.Title className="sr-only">Dialog</Dialog.Title>}
            {children}
            {!hideFooter && (
              <div className="mt-modal-footer flex justify-end gap-2 p-2">{footerContent}</div>
            )}
          </Dialog.Content>
        </Dialog.Overlay>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

function Content({ children }: { children: React.ReactNode }) {
  return <div className="px-4">{children}</div>
}

function Separator() {
  return <div className="w-full h-px  my-2"></div>
}

Modal.Content = Content
Modal.Separator = Separator
export { Modal }
