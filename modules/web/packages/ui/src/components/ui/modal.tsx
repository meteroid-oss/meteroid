import * as Dialog from '@radix-ui/react-dialog'
import { VariantProps, cva } from 'class-variance-authority'
import { useEffect, useState } from 'react'

import { cn } from '@ui/lib'

import { Button } from './button'
import { Spinner } from './spinner'

const variants = cva(
  'relative bg-popover my-4 border rounded-xl shadow-xl data-open:animate-overlay-show data-closed:animate-overlay-hide',
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
  ...props
}: ModalProps & VariantProps<typeof variants>) => {
  const [open, setOpen] = useState(visible ? visible : false)

  useEffect(() => {
    setOpen(visible)
  }, [visible])

  const footerContent = customFooter ? (
    customFooter
  ) : (
    <div className="flex w-full space-x-2 justify-end">
      <Button variant="secondary" onClick={onCancel} disabled={loading} size="sm">
        {cancelText}
      </Button>
      <Button onClick={onConfirm} disabled={loading} hasIcon={loading} size="sm">
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
          bg-black
          h-full w-full
          left-0
          top-0
          opacity-75
          data-closed:animate-fade-out-overlay-bg
          data-open:animate-fade-in-overlay-bg"
        />
        <Dialog.Overlay
          className="z-50
          fixed
          inset-0
          grid
          place-items-center
          overflow-y-auto
          data-open:animate-overlay-show data-closed:animate-overlay-hide"
        >
          <Dialog.Content
            className={cn(variants({ size }), className)}
            onInteractOutside={props.onInteractOutside}
            onEscapeKeyDown={props.onEscapeKeyDown}
          >
            {header && (
              <div className=" space-y-1 py-3 px-4 sm:px-5 rounded-xl rounded-b-none border-b border-border">
                {header}
              </div>
            )}
            {children}
            {!hideFooter && (
              <div className="flex justify-end gap-2 py-3 px-5 border-t ">{footerContent}</div>
            )}
          </Dialog.Content>
        </Dialog.Overlay>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

function Content({ children }: { children: React.ReactNode }) {
  return <div className="px-5">{children}</div>
}

function Separator() {
  return <div className="w-full h-px  my-2"></div>
}

Modal.Content = Content
Modal.Separator = Separator
export { Modal }
