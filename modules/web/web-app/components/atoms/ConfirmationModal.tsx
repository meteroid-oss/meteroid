import { ButtonAlt, Modal } from '@md/ui'
import { FC, MouseEventHandler, useEffect, useState } from 'react'

interface Props {
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

const ConfirmationModal: FC<Props> = ({
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
          <ButtonAlt type="default" disabled={loading} onClick={onSelectCancel}>
            Cancel
          </ButtonAlt>
          <ButtonAlt
            type={danger ? 'danger' : 'primary'}
            loading={loading}
            disabled={loading}
            onClick={onConfirm}
          >
            {loading ? buttonLoadingLabel : buttonLabel}
          </ButtonAlt>
        </div>
      }
    >
      {children}
    </Modal>
  )
}

export default ConfirmationModal
