import { Container } from '@ui/components'
import { PropsWithChildren, ReactNode } from 'react'
import { Outlet } from 'react-router-dom'

import { LayoutHeader } from '../shared/LayoutHeader'

import { NavigationBar } from './NavigationBar/NavigationBar'
import InnerMenu from './components/InnerMenu'

interface TenantLayoutProps {
  title: string
  isLoading?: boolean
  innerMenu?: ReactNode
  hideHeader?: boolean
}

export const TenantPageLayout = ({
  title,
  innerMenu,
  children,
  hideHeader = false,
}: PropsWithChildren<TenantLayoutProps>) => {
  return (
    <>
      {innerMenu && <InnerMenu title={title}>{innerMenu}</InnerMenu>}

      <main className="flex flex-col flex-1 w-full h-full overflow-x-hidden ">
        {!hideHeader && <LayoutHeader />}
        <Container fullHeight>{children}</Container>
      </main>
    </>
  )
}

interface TenantLayoutOutletProps {
  hideIconBar?: boolean
}
export const TenantLayoutOutlet = ({
  hideIconBar = false,
}: PropsWithChildren<TenantLayoutOutletProps>) => {
  return (
    <div className="flex h-full">
      {/* Left-most navigation side bar */}
      {!hideIconBar && <NavigationBar />}

      <Outlet />
    </div>
  )
}
