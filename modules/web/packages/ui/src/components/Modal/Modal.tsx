import * as Dialog from '@radix-ui/react-dialog'
import { useEffect, useState } from 'react'

import { ButtonAlt as Button } from '../ButtonAlt'

import { twModalStyles } from './Modal.styles'

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
  variant?: 'danger' | 'warning' | 'success'
  visible: boolean
  size?: 'tiny' | 'small' | 'medium' | 'large' | 'xlarge' | 'xxlarge'
  className?: string
  triggerElement?: React.ReactNode
  header?: React.ReactNode
}

const Modal = ({
  children,
  customFooter = undefined,
  hideFooter = false,
  alignFooter = 'left',
  layout = 'horizontal',
  loading = false,
  cancelText = 'Cancel',
  onConfirm = () => {},
  onCancel = () => {},
  confirmText = 'Confirm',
  variant = 'success',
  visible = false,
  size = 'large',
  className = '',
  triggerElement,
  header,
  ...props
}: ModalProps) => {
  const [open, setOpen] = useState(visible ? visible : false)

  const __styles = twModalStyles.modal

  useEffect(() => {
    setOpen(visible)
  }, [visible])

  const footerContent = customFooter ? (
    customFooter
  ) : (
    <div className="flex w-full space-x-2 justify-end">
      <Button type="default" onClick={onCancel} disabled={loading}>
        {cancelText}
      </Button>
      <Button
        onClick={onConfirm}
        disabled={loading}
        loading={loading}
        type={variant === 'danger' ? 'danger' : 'primary'}
      >
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
        <Dialog.Overlay className={__styles.overlay} />
        <Dialog.Overlay className={__styles.scroll_overlay}>
          <Dialog.Content
            className={[__styles.base, __styles.size[size], className].join(' ')}
            onInteractOutside={props.onInteractOutside}
            onEscapeKeyDown={props.onEscapeKeyDown}
          >
            {header && <div className={__styles.header}>{header}</div>}
            {children}
            {!hideFooter && <div className={__styles.footer}>{footerContent}</div>}
          </Dialog.Content>
        </Dialog.Overlay>
      </Dialog.Portal>
    </Dialog.Root>
  )
}

function Content({ children }: { children: React.ReactNode }) {
  const __styles = twModalStyles.modal

  return <div className={__styles.content}>{children}</div>
}

export function Separator() {
  const __styles = twModalStyles.modal

  return <div className={__styles.separator}></div>
}

Modal.Content = Content
Modal.Separator = Separator
export default Modal
