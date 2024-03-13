import { ScrollArea } from '@md/ui'
import { PropsWithChildren, ReactNode } from 'react'
import { Outlet } from 'react-router-dom'

import { LayoutHeader } from '../shared/LayoutHeader'

import { NavigationBar } from './NavigationBar/NavigationBar'
import InnerMenu from './components/InnerMenu'

interface TenantLayoutProps {
  title: string
  isLoading?: boolean
  familyPicker?: boolean
  innerMenu?: ReactNode
  hideHeader?: boolean
  displayTitle?: boolean
}

export const TenantPageLayout = ({
  title,
  innerMenu,
  children,
  hideHeader = false,
  familyPicker = false,
  displayTitle = false,
}: PropsWithChildren<TenantLayoutProps>) => {
  return (
    <>
      {innerMenu && <InnerMenu title={title}>{innerMenu}</InnerMenu>}

      <main className="flex flex-col flex-1 w-full max-w-[1620px] mx-auto h-full overflow-x-hidden ">
        {!hideHeader && (
          <LayoutHeader familyPicker={familyPicker} title={displayTitle ? title : undefined} />
        )}
        <ScrollArea className="relative px-10 py-8">{children}</ScrollArea>
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
