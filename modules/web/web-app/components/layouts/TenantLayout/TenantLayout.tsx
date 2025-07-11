import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Flex,
  Separator,
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  cn,
  useSidebar,
} from '@ui/index'
import { ChevronDown, Plus } from 'lucide-react'
import { PropsWithChildren } from 'react'
import { NavLink, Outlet, useLocation } from 'react-router-dom'

import { NavMain } from '@/components/layouts/TenantLayout/components/NavMain'
import { sidebarItems } from '@/components/layouts/TenantLayout/utils'
import { TenantDropdown } from '@/components/layouts/shared/LayoutHeader/TenantDropdown'
import { useLogout } from '@/hooks/useLogout'
import { useQuery } from '@/lib/connectrpc'
import { getCurrentOrganizations } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { OrganizationUserRole } from '@/rpc/api/users/v1/models_pb'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const TenantPageLayout = ({ children }: PropsWithChildren) => {
  return (
    <>
      <main className="flex  flex-col flex-1 w-full max-w-screen-2xl mx-auto h-full overflow-x-hidden">
        <div className="relative pt-4 px-6 h-full overflow-y-auto flex flex-col gap-5">
          {children}
        </div>
      </main>
    </>
  )
}

export const TenantLayoutOutlet = () => {
  const { pathname } = useLocation()

  const logout = useLogout()
  const meData = useQuery(me)?.data
  const organizationData = useQuery(getCurrentOrganizations)?.data?.organization

  const { toggleSidebar, state } = useSidebar()

  const isCollapsed = state === 'collapsed'

  function mapRole(currentOrganizationRole: OrganizationUserRole | undefined): string {
    switch (currentOrganizationRole) {
      case OrganizationUserRole.ADMIN:
        return 'Admin'
      case OrganizationUserRole.MEMBER:
        return 'Member'
      case undefined:
      default:
        return ''
    }
  }

  return (
    <>
      <Sidebar collapsible="icon">
        <SidebarHeader>
          <SidebarMenu>
            <SidebarMenuItem>
              {isCollapsed ? (
                <div
                  className="flex aspect-square h-5 w-5 rounded-md ml-1.5"
                  style={{
                    background: `linear-gradient(0deg, #C7B3FE, #C7B3FE), 
                linear-gradient(0deg, #B69EF0, #B69EF0)`,
                  }}
                />
              ) : (
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <SidebarMenuButton asChild className="cursor-pointer w-fit h-[28px] pl-1.5">
                      <Flex align="center" className="gap-2">
                        <div
                          className="flex aspect-square h-5 w-5 rounded-md"
                          style={{
                            background: `linear-gradient(0deg, #C7B3FE, #C7B3FE), 
                      linear-gradient(0deg, #B69EF0, #B69EF0)`,
                          }}
                        />
                        <span className="font-semibold ml-1 text-foreground">
                          {organizationData?.tradeName ?? ''}
                        </span>
                        <ChevronDown className="text-muted-foreground" />
                      </Flex>
                    </SidebarMenuButton>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="start" className="p-0 w-56 ">
                    <DropdownMenuItem className="pointer-events-none">
                      <div
                        className="flex aspect-square h-8 w-8 rounded-md"
                        style={{
                          background: `linear-gradient(0deg, #C7B3FE, #C7B3FE), 
                      linear-gradient(0deg, #B69EF0, #B69EF0)`,
                        }}
                      />
                      <Flex direction="column" className="ml-2">
                        <div>{organizationData?.tradeName ?? ''}</div>
                        <div className="text-xs text-secondary-foreground">
                          {mapRole(meData?.currentOrganizationRole)}
                        </div>
                      </Flex>
                    </DropdownMenuItem>
                    <DropdownMenuItem>
                      <Plus size={16} className="mr-2" />
                      New organization
                    </DropdownMenuItem>
                    <Separator />

                    <DropdownMenuItem className="pointer-events-none text-xs text-muted-foreground flex">
                      {meData?.user?.email}
                    </DropdownMenuItem>
                    <Separator />
                    <DropdownMenuItem onClick={() => logout('User clicked on logout')}>
                      Log out
                    </DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
              )}
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarHeader>
        <SidebarContent>
          <Flex justify="center" align="center" className="px-3 w-full mt-2">
            <TenantDropdown />
          </Flex>
          <NavMain items={sidebarItems.mainNav} />
        </SidebarContent>
        <div>
          <SidebarGroup>
            <SidebarGroupContent>
              <SidebarMenu>
                {sidebarItems.navSecondary.map(item => (
                  <SidebarMenuItem key={item.title}>
                    <NavLink to={item.url} viewTransition>
                      <SidebarMenuButton isActive={pathname.includes(item.url)} asChild size="sm">
                        <Flex align="center" className="gap-2">
                          <item.icon />
                          <span>{item.title}</span>
                        </Flex>
                      </SidebarMenuButton>
                    </NavLink>
                  </SidebarMenuItem>
                ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        </div>
        <button
          onClick={toggleSidebar}
          className={cn(
            'absolute z-30 -right-6 top-1/2 -translate-y-1/2 h-16 w-6 flex items-center justify-center',
            isCollapsed ? 'cursor-e-resize' : 'cursor-w-resize'
          )}
          aria-label="Toggle Sidebar"
        >
          <div className="h-16 w-1 rounded-full bg-sidebar-border/80 hover:bg-sidebar-border transition-colors" />
        </button>
      </Sidebar>
      <SidebarInset>
        <Outlet />
      </SidebarInset>
    </>
  )
}
