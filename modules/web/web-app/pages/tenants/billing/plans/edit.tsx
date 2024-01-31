import { FunctionComponent } from 'react'
import { Outlet } from 'react-router-dom'

import { PlanBuilder } from '@/features/billing/plans/PlanBuilder'

export const PlanEdit: FunctionComponent = () => {
  // const setEditPanelVisible = () => navigate('new')

  return (
    <>
      <PlanBuilder>
        <Outlet />
      </PlanBuilder>
    </>
  )
}
