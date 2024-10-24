import { Badge, Modal } from '@md/ui'
import { FC } from 'react'

import { DetailsForm } from '@/features/billing/plans/create/details/DetailsForm'
import { useProductFamily } from '@/hooks/useProductFamily'

interface Props {
  modalVisible: boolean
  setModalVisible: (visible: boolean) => void
}
export const PlanCreateInitModal: FC<Props> = ({ modalVisible, setModalVisible }) => {
  const onSelectCancel = () => {
    //TODO methods.reset
    setModalVisible(false)
  }

  const { productFamily } = useProductFamily()

  return (
    <Modal
      layout="vertical"
      visible={modalVisible}
      header={
        <>
          <>Create a new plan in product line </>
          <Badge variant="outline">{productFamily?.name}</Badge>
        </>
      }
      size="xlarge"
      onCancel={onSelectCancel}
      hideFooter
    >
      <div className="px-5 py-4">
        <DetailsForm onCancel={onSelectCancel} />
      </div>
    </Modal>
  )
}
