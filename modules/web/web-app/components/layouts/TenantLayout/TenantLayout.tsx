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

      <main className="flex  flex-col flex-1 w-full max-w-screen-2xl pl-8 pr-2 mx-auto h-full overflow-x-hidden ">
        {!hideHeader && (
          <div className="px-4">
            <LayoutHeader familyPicker={familyPicker} title={displayTitle ? title : undefined} />
          </div>
        )}
        <ScrollArea className="relative py-8 px-4 h-full">
          <div>{children}</div>
        </ScrollArea>
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
