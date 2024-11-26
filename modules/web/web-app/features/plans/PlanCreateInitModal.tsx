import { Modal } from '@md/ui'

import { DetailsForm } from '@/features/plans/create/details/DetailsForm'
import { useNavigate } from 'react-router-dom'

export const PlanCreateInitModal = () => {
  const navigate = useNavigate()
  const onSelectCancel = () => {
    //TODO methods.reset
    navigate('..')
  }

  return (
    <Modal
      layout="vertical"
      visible={true}
      header={<>Create a new plan </>}
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
