import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Flex,
  Separator,
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  useSidebar,
} from '@ui/index'
import { ChevronDown, Menu, PanelLeftClose, PanelLeftOpen, Plus } from 'lucide-react'
import { PropsWithChildren, useEffect } from 'react'
import { NavLink, useLocation, useNavigate } from 'react-router-dom'

import { NavMain } from '@/components/layouts/TenantLayout/components/NavMain'
import { OverlayOutlet } from '@/components/layouts/TenantLayout/components/OverlayOutlet'
import { getFilteredSidebarItems } from '@/components/layouts/TenantLayout/utils'
import { TenantDropdown } from '@/components/layouts/shared/LayoutHeader/TenantDropdown'
import { SeedingBanner } from '@/components/layouts/shared/SeedingBanner'
import { TenantStatusBanner } from '@/components/layouts/shared/TenantStatusBanner'
import { TestModeBanner } from '@/components/layouts/shared/TestModeBanner'
import { Slot, slotItems, useNavMainSections, useVisibleItems } from '@/edition'
import UpdatesPopover from '@/features/updates/UpdatesPopover'
import { useTrackLastVisited } from '@/features/updates/useUserPreferences'
import { useLogout } from '@/hooks/useLogout'
import { usePermissions } from '@/hooks/usePermissions'
import { useTenant } from '@/hooks/useTenant'
import { useQuery } from '@/lib/connectrpc'
import { setCreditUnitPrecisions, setCurrencyPrecisions } from '@/lib/data/currency-precision'
import { useTypedParams } from '@/lib/utils/params'
import { listCreditUnits } from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'
import {
  getCurrencies,
  getInstance,
} from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { getCurrentOrganizations } from '@/rpc/api/organizations/v1/organizations-OrganizationsService_connectquery'
import { OrganizationUserRole } from '@/rpc/api/users/v1/models_pb'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const TenantPageLayout = ({ children }: PropsWithChildren) => {
  return (
    <>
      {/* The window is the scroller (nothing here has a constrained height), so neither element
          may become a scroll container: `overflow-x-clip` clips without creating one, keeping
          `position: sticky` (e.g. a list card's footer) relative to the viewport. */}
      <main className="flex flex-col flex-1 w-full max-w-screen-2xl mx-auto h-full overflow-x-clip pb-4">
        <div className="relative pt-4 px-6 h-full flex flex-col gap-5">{children}</div>
      </main>
    </>
  )
}

export const TenantLayoutOutlet = () => {
  const { pathname } = useLocation()
  const { organizationSlug, tenantSlug } = useTypedParams()
  useTrackLastVisited(organizationSlug, tenantSlug)

  const navigate = useNavigate()
  const logout = useLogout()
  const meData = useQuery(me)?.data
  const orgResponse = useQuery(getCurrentOrganizations)?.data
  const organizationData = orgResponse?.organization
  const getInstanceQuery = useQuery(getInstance)
  const { tenant } = useTenant()

  // Seed the currency-precision cache from the backend (single source of truth) so the money
  // formatting helpers use server-authoritative minor-unit precision.
  const currenciesData = useQuery(getCurrencies)?.data
  useEffect(() => {
    if (currenciesData) setCurrencyPrecisions(currenciesData.currencies)
  }, [currenciesData])
  // Credit unit wallets and budgets format their amounts with the same helpers.
  const creditUnitsData = useQuery(listCreditUnits, { includeArchived: true })?.data
  useEffect(() => {
    if (creditUnitsData) setCreditUnitPrecisions(creditUnitsData.units)
  }, [creditUnitsData])

  const { toggleSidebar, state, isMobile, setOpenMobile } = useSidebar()

  const isExpress = organizationData?.isExpress ?? false
  const { has, isLoading: permsLoading } = usePermissions()
  // While permissions load, show everything to avoid a flash of empty nav.
  const canView = (code: string) => permsLoading || has(code)
  const moduleNavSecondary = useVisibleItems(slotItems('layout.navSecondary'), undefined)
  const moduleNavMain = useNavMainSections()
  const filteredSidebarItems = getFilteredSidebarItems(
    isExpress,
    canView,
    moduleNavSecondary,
    moduleNavMain
  )

  // The longest match wins, so `settings/enterprise` doesn't also light up Settings.
  const footerItems = [...filteredSidebarItems.navSecondary, ...filteredSidebarItems.navTrailing]
  const activeSecondaryUrl = footerItems
    .map(item => item.url)
    .filter(url => pathname.includes(url))
    .sort((a, b) => b.length - a.length)[0]

  const renderFooterItem = (item: (typeof footerItems)[number]) => (
    <SidebarMenuItem key={item.title}>
      <NavLink to={item.url}>
        <SidebarMenuButton isActive={item.url === activeSecondaryUrl} asChild size="sm">
          <Flex align="center" className="gap-2">
            <item.icon />
            <span>{item.title}</span>
          </Flex>
        </SidebarMenuButton>
      </NavLink>
    </SidebarMenuItem>
  )

  const isCollapsed = state === 'collapsed'

  // On mobile the sidebar is an off-canvas drawer; close it whenever the route changes
  useEffect(() => {
    if (isMobile) setOpenMobile(false)
  }, [pathname, isMobile, setOpenMobile])

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

  if (meData?.user?.onboarded === false) {
    navigate('/onboarding/user')
  }

  return (
    <>
      <Sidebar
        collapsible="icon"
        className="[@media(max-height:900px)]:[&_[data-sidebar=menu-button]]:h-7 [@media(max-height:900px)]:[&_[data-sidebar=menu-button]]:py-1 [@media(max-height:900px)]:[&_[data-sidebar=menu-button]]:text-[13px]"
      >
        <SidebarHeader>
          <SidebarMenu>
            <SidebarMenuItem>
              {isCollapsed ? (
                <Flex direction="column" align="center" className="gap-1.5">
                  <div
                    className="flex aspect-square h-5 w-5 rounded-md"
                    style={{
                      background: `linear-gradient(0deg, #C7B3FE, #C7B3FE),
                linear-gradient(0deg, #B69EF0, #B69EF0)`,
                    }}
                  />
                  <button
                    onClick={toggleSidebar}
                    aria-label="Expand sidebar"
                    title="Expand sidebar"
                    className="flex h-6 w-6 items-center justify-center rounded-md text-muted-foreground hover:bg-sidebar-accent hover:text-foreground"
                  >
                    <PanelLeftOpen className="h-4 w-4" />
                  </button>
                </Flex>
              ) : (
                <Flex align="center" className="gap-1">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <SidebarMenuButton
                        asChild
                        className="cursor-pointer flex-1 min-w-0 h-[28px] pl-1.5"
                      >
                        <Flex align="center" className="gap-2">
                          <div
                            className="flex aspect-square h-5 w-5 shrink-0 rounded-md"
                            style={{
                              background: `linear-gradient(0deg, #C7B3FE, #C7B3FE),
                      linear-gradient(0deg, #B69EF0, #B69EF0)`,
                            }}
                          />
                          <span
                            className="font-semibold ml-1 text-foreground truncate min-w-0"
                            title={organizationData?.tradeName ?? ''}
                          >
                            {organizationData?.tradeName ?? ''}
                          </span>
                          <ChevronDown className="text-muted-foreground shrink-0" />
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
                        <Flex direction="column" className="ml-2 min-w-0">
                          <div className="truncate" title={organizationData?.tradeName ?? ''}>
                            {organizationData?.tradeName ?? ''}
                          </div>
                          <div className="text-xs text-secondary-foreground">
                            {mapRole(orgResponse?.role)}
                          </div>
                        </Flex>
                      </DropdownMenuItem>
                      <Separator />
                      {meData?.organizations && meData.organizations.length > 1 && (
                        <>
                          {meData.organizations
                            .filter(org => org.slug !== organizationData?.slug)
                            .map(org => (
                              <DropdownMenuItem
                                key={org.id}
                                className="cursor-pointer"
                                onClick={() => {
                                  window.location.href = `/${org.slug}`
                                }}
                              >
                                <div
                                  className="flex aspect-square h-5 w-5 rounded-md mr-2"
                                  style={{
                                    background: `linear-gradient(0deg, #C7B3FE, #C7B3FE),
                                linear-gradient(0deg, #B69EF0, #B69EF0)`,
                                  }}
                                />
                                {org.tradeName}
                              </DropdownMenuItem>
                            ))}
                          <Separator />
                        </>
                      )}
                      {!isExpress && getInstanceQuery?.data?.multiOrganizationEnabled && (
                        <>
                          <DropdownMenuItem
                            className="cursor-pointer"
                            onClick={() => navigate('/onboarding/organization')}
                          >
                            <Plus size={16} className="mr-2" />
                            New workspace
                          </DropdownMenuItem>
                          <Separator />
                        </>
                      )}

                      <DropdownMenuItem className="pointer-events-none text-xs ml-1 block">
                        <div>{meData?.user?.firstName}</div>
                        <div className="text-muted-foreground font-medium  truncate max-w-[200px]">
                          {meData?.user?.email}
                        </div>
                      </DropdownMenuItem>
                      <Separator />
                      <DropdownMenuItem onClick={() => logout('User clicked on logout')}>
                        Log out
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                  <button
                    onClick={toggleSidebar}
                    aria-label="Collapse sidebar"
                    title="Collapse sidebar"
                    className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-sidebar-accent hover:text-foreground"
                  >
                    <PanelLeftClose className="h-4 w-4" />
                  </button>
                </Flex>
              )}
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarHeader>
        <SidebarContent>
          {!isExpress && (
            <Flex justify="center" align="center" className="px-3 w-full mt-2">
              <TenantDropdown />
            </Flex>
          )}
          {!isExpress && <Slot name="layout.sidebarActions" />}
          <NavMain items={filteredSidebarItems.mainNav} />
        </SidebarContent>
        <SidebarFooter>
          <SidebarGroup>
            <SidebarGroupContent>
              <SidebarMenu>
                <UpdatesPopover />
                {filteredSidebarItems.navSecondary.map(renderFooterItem)}
                {filteredSidebarItems.navTrailing.map(renderFooterItem)}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="relative">
        {/* Mobile-only top bar: the sidebar is an off-canvas drawer below md, so this
            is the only way to open it on touch screens */}
        <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-3 md:hidden">
          <button
            onClick={toggleSidebar}
            aria-label="Open navigation menu"
            className="flex h-9 w-9 items-center justify-center rounded-md text-foreground hover:bg-sidebar-accent"
          >
            <Menu className="h-5 w-5" />
          </button>
          <span className="truncate font-medium" title={organizationData?.tradeName ?? ''}>
            {organizationData?.tradeName ?? ''}
          </span>
        </header>
        <TestModeBanner environment={tenant?.environment} />
        <SeedingBanner tenantId={tenant?.id} environment={tenant?.environment} />
        <div className="pointer-events-none absolute top-0 left-0 right-0 z-30 flex flex-col">
          <Slot name="layout.banners" />
          <TenantStatusBanner tenantId={tenant?.id} status={tenant?.status} />
        </div>
        <OverlayOutlet />
      </SidebarInset>
    </>
  )
}
