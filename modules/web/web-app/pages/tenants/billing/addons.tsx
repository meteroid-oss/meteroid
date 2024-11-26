import { FunctionComponent } from 'react'

import { AddonsPage } from '@/features/productCatalog/addons/AddonsPage'
import { Outlet } from 'react-router'

export const Addons: FunctionComponent = () => {
  return (
    <>
      <AddonsPage />
      <Outlet />
    </>
  )
}
