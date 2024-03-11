import { Button, Modal } from '@ui2/components'
import { FC, MouseEventHandler, useEffect, useState } from 'react'

export interface ConfirmationModalProps {
  visible: boolean
  danger?: boolean
  header: string | JSX.Element
  size?: 'small' | 'tiny' | 'medium' | 'large'
  buttonLabel: string
  buttonLoadingLabel?: string
  onSelectCancel: () => void
  onSelectConfirm: () => void
  children?: React.ReactNode
}

const ConfirmationModal: FC<ConfirmationModalProps> = ({
  visible = false,
  danger = false,
  header = '',
  size = 'small',
  buttonLabel = '',
  buttonLoadingLabel = '',
  onSelectCancel = () => {},
  onSelectConfirm = () => {},
  children,
}) => {
  useEffect(() => {
    if (visible) {
      setLoading(false)
    }
  }, [visible])

  const [loading, setLoading] = useState(false)

  const onConfirm: MouseEventHandler<HTMLButtonElement> = e => {
    e.preventDefault()
    e.stopPropagation()
    setLoading(true)
    onSelectConfirm()
  }

  return (
    <Modal
      layout="vertical"
      visible={visible}
      header={header}
      size={size}
      onCancel={onSelectCancel}
      customFooter={
        <div className="flex justify-end w-full items-center space-x-3">
          <Button variant="secondary" disabled={loading} onClick={onSelectCancel}>
            Cancel
          </Button>
          <Button
            variant={danger ? 'destructive' : 'alternative'}
            disabled={loading}
            onClick={onConfirm}
          >
            {loading ? buttonLoadingLabel : buttonLabel}
          </Button>
        </div>
      }
    >
      {children}
    </Modal>
  )
}

export default ConfirmationModal
