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
  displayTitle?: boolean
}

export const TenantPageLayout = ({
  title,
  innerMenu,
  children,
  hideHeader = false,
  displayTitle = false,
}: PropsWithChildren<TenantLayoutProps>) => {
  return (
    <>
      {innerMenu && <InnerMenu title={title}>{innerMenu}</InnerMenu>}

      <main className="flex  flex-col flex-1 w-full max-w-screen-2xl pl-8 pr-2 mx-auto h-full overflow-x-hidden ">
        {!hideHeader && (
          <div className="px-4">
            <LayoutHeader title={displayTitle ? title : undefined} />
          </div>
        )}
        <div className="scrollbar relative py-4 px-4 h-full overflow-y-auto flex flex-col gap-5">
          {children}
        </div>
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
