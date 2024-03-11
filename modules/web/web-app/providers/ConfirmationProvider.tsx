import ConfirmationModal, { ConfirmationModalProps } from '@/components/ConfirmationModal'
import { Modal } from '@ui2/components'
import { createContext, FC, ReactNode, useContext, useState } from 'react'

interface ConfirmationProps {
  message?: string | JSX.Element
  header?: string | JSX.Element
}

interface ConfirmationModalContextType {
  showConfirmModal: (
    confirm: () => void,
    contentOptions?: ConfirmationProps,
    options?: Partial<Omit<ConfirmationModalProps, 'visible' | 'header'>>
  ) => void
}

const ConfirmationModalContext = createContext<ConfirmationModalContextType | undefined>(undefined)

/**
 * Example usage :
 * ```tsx
 *  const showConfirmModal = useConfirmationModal()
 *  const handleDelete = () => {
 *      showConfirmModal(() => {
 *      // delete logic
 *      }, { // optional
 *          message: 'Are you sure you want to delete this item ?',
 *          header: 'Delete item'
 *      })
 *  }
 * ```
 */
export const useConfirmationModal = () => {
  const context = useContext(ConfirmationModalContext)
  if (!context) {
    throw new Error('useConfirmationModal must be used within a ConfirmationModalProvider')
  }
  return context.showConfirmModal
}

const DefaultModalContent = ({ message }: { message?: string | JSX.Element }) => (
  <Modal.Content>
    <p className="py-4 text-sm text-muted-foreground">
      {message || 'This action cannot be undone. Are you sure you want to continue ?'}
    </p>
  </Modal.Content>
)

const ConfirmationModalProvider: FC<{ children: ReactNode }> = ({ children }) => {
  const [modalProps, setModalProps] = useState<ConfirmationModalProps | null>(null)

  const showConfirmModal: ConfirmationModalContextType['showConfirmModal'] = (
    confirm,
    content,
    options
  ) => {
    setModalProps({
      ...options,
      buttonLabel: options?.buttonLabel || 'Confirm',
      header: content?.header || 'Warning',
      danger: options?.danger || true,
      children: options?.children || <DefaultModalContent message={content?.message} />,
      onSelectCancel: options?.onSelectCancel || hideModal,
      onSelectConfirm: confirm,
      visible: true,
    })
  }

  const hideModal = () => {
    setModalProps(null)
  }

  return (
    <ConfirmationModalContext.Provider value={{ showConfirmModal }}>
      {children}
      {modalProps && <ConfirmationModal {...modalProps} />}
    </ConfirmationModalContext.Provider>
  )
}

export default ConfirmationModalProvider
