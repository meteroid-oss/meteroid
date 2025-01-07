import { FunctionComponent } from 'react'
import { Outlet } from 'react-router'

import { AddonsPage } from '@/features/productCatalog/addons/AddonsPage'

export const Addons: FunctionComponent = () => {
  return (
    <>
      <AddonsPage />
      <Outlet />
    </>
  )
}
