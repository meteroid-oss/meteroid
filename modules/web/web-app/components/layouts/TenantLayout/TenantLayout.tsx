import {
  cn,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Flex,
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  useSidebar,
} from '@ui/index'
import {
  FileSignature,
  Gauge,
  Layers,
  Menu,
  Plus,
  ReceiptText,
  Repeat,
  TicketPercent,
  UserRound,
} from 'lucide-react'
import { Fragment, PropsWithChildren, useEffect } from 'react'
import { NavLink, useLocation, useNavigate } from 'react-router'

import {
  ChevronsUpDownIcon,
  InitialAvatar,
  NewIcon,
  SidebarPanelIcon,
  WorkspaceMark,
} from '@/components/icons/AppIcons'
import { PageIconContext, resolvePageIcon } from '@/components/layouts/TenantLayout/PageIconContext'
import { NavMain } from '@/components/layouts/TenantLayout/components/NavMain'
import { OverlayOutlet } from '@/components/layouts/TenantLayout/components/OverlayOutlet'
import { getFilteredSidebarItems } from '@/components/layouts/TenantLayout/utils'
import { EnvironmentBanner } from '@/components/layouts/shared/EnvironmentBanner'
import { TenantDropdown } from '@/components/layouts/shared/LayoutHeader/TenantDropdown'
import { SeedingBanner } from '@/components/layouts/shared/SeedingBanner'
import { TenantStatusBanner } from '@/components/layouts/shared/TenantStatusBanner'
import {
  SIDEBAR_MENU_CONTENT,
  SIDEBAR_MENU_ITEM,
  SIDEBAR_MENU_SEPARATOR,
} from '@/components/layouts/shared/sidebarMenuStyles'
import { Slot, slotItems, useNavMainSections, useVisibleItems } from '@/edition'
import { LaunchPad } from '@/features/guide/LaunchPad'
import { useGuidePrefs } from '@/features/guide/useGuidePrefs'
import { usePrefetchListViews } from '@/features/listviews/useListViews'
import { useTrackLastVisited } from '@/features/updates/useUserPreferences'
import { useBasePath } from '@/hooks/useBasePath'
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
import { DARK_VARIANTS, DARK_VARIANT_LABELS, useTheme } from 'providers/ThemeProvider'

export const TenantPageLayout = ({ children }: PropsWithChildren) => {
  return (
    <>
      {/* The window is the scroller (nothing here has a constrained height), so neither element
          may become a scroll container: `overflow-x-clip` clips without creating one, keeping
          `position: sticky` relative to the viewport. A list page's table card is the exception
          (`data-fit-viewport`): the page is held to the window's height and the rows scroll
          inside the card, so `min-h-0` lets each level shrink below its content. */}
      {/* A full-bleed page (`data-full-bleed`, e.g. the list pages) runs edge to edge and lays out
          its own spacing; a list page's card reaches the window's bottom edge. */}
      <main className="flex flex-col flex-1 w-full max-w-screen-2xl mx-auto h-full min-h-0 overflow-x-clip pb-4 has-[[data-full-bleed]]:max-w-none has-[[data-full-bleed]]:pb-0">
        <div className="relative pt-4 px-6 h-full flex flex-col gap-5 has-[[data-full-bleed]]:flex-1 has-[[data-fit-viewport]]:h-auto has-[[data-fit-viewport]]:min-h-0 has-[[data-full-bleed]]:p-0 has-[[data-full-bleed]]:gap-0">
          {children}
        </div>
      </main>
    </>
  )
}

// What "New" can start, in groups: the customer, what they're billed, what they're sold.
const NEW_GROUPS = [
  [{ title: 'Customer', url: 'customers?create=1', icon: UserRound }],
  [
    { title: 'Subscription', url: 'subscriptions/create', icon: Repeat },
    { title: 'Invoice', url: 'invoices/create', icon: ReceiptText },
    { title: 'Quote', url: 'quotes/create', icon: FileSignature },
  ],
  [
    { title: 'Plan', url: 'plans/add-plan', icon: Layers },
    { title: 'Metric', url: 'metrics/add-metric', icon: Gauge },
    { title: 'Coupon', url: 'coupons/add-coupon', icon: TicketPercent },
  ],
]

const NewMenu = () => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  return (
    <SidebarMenuItem>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <SidebarMenuButton tooltip="New" className="group/nav cursor-pointer">
            <span className="flex size-5 shrink-0 items-center justify-center text-faint-foreground group-hover/nav:text-sidebar-accent-foreground">
              <NewIcon />
            </span>
            <span>New</span>
          </SidebarMenuButton>
        </DropdownMenuTrigger>
        {/* The sidebar's own menu look, as the organisation and tenant menus. */}
        <DropdownMenuContent
          align="start"
          side="bottom"
          className={cn('w-48', SIDEBAR_MENU_CONTENT)}
        >
          {NEW_GROUPS.map((group, index) => (
            <Fragment key={index}>
              {index > 0 && <DropdownMenuSeparator className={SIDEBAR_MENU_SEPARATOR} />}
              {group.map(item => (
                <DropdownMenuItem
                  key={item.url}
                  className={cn('group/new', SIDEBAR_MENU_ITEM)}
                  onSelect={() => navigate(`${basePath}/${item.url}`)}
                >
                  {/* Quiet at rest, as the nav's icons; full contrast on the item in focus. */}
                  <item.icon
                    size={16}
                    strokeWidth={1.5}
                    aria-hidden
                    className="shrink-0 text-sidebar-icon transition-colors group-focus/new:text-sidebar-accent-foreground"
                  />
                  {item.title}
                </DropdownMenuItem>
              ))}
            </Fragment>
          ))}
        </DropdownMenuContent>
      </DropdownMenu>
    </SidebarMenuItem>
  )
}

export const TenantLayoutOutlet = () => {
  const { pathname } = useLocation()
  const { organizationSlug, tenantSlug } = useTypedParams()
  useTrackLastVisited(organizationSlug, tenantSlug)
  usePrefetchListViews(tenantSlug)

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
  const { isDarkMode, toggleTheme, darkVariant, setDarkVariant } = useTheme()
  const guidePrefs = useGuidePrefs()
  const basePath = useBasePath()

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

  // The page header shows the icon of the nav entry you are in, where Home used to be.
  const pageIcon = resolvePageIcon(pathname, [...filteredSidebarItems.mainNav, ...footerItems])

  // Settings, help and the like sit as icons on the footer row, named by their tooltip.
  const renderFooterItem = (item: (typeof footerItems)[number]) => {
    const active = item.url === activeSecondaryUrl
    return (
      <Tooltip key={item.title}>
        <TooltipTrigger asChild>
          <NavLink
            to={item.url}
            aria-label={item.title}
            aria-current={active ? 'page' : undefined}
            className={cn(
              'relative isolate flex size-5 items-center justify-center transition-colors duration-150 before:absolute before:-inset-1 before:-z-10 before:rounded-[5px] before:transition-[background-color,scale] before:duration-150 hover:text-sidebar-accent-foreground hover:before:bg-sidebar-accent motion-safe:active:before:scale-[0.9] [&>svg]:max-h-4 [&>svg]:max-w-4',
              active ? 'text-sidebar-accent-foreground' : 'text-faint-foreground'
            )}
          >
            <item.icon />
          </NavLink>
        </TooltipTrigger>
        {/* Collapsed, the footer icons stack, so their names show beside them. */}
        <TooltipContent side={isCollapsed ? 'right' : 'top'}>{item.title}</TooltipContent>
      </Tooltip>
    )
  }

  const isCollapsed = state === 'collapsed'

  // The organisation menu: under the org name, or beside the mark when the nav is collapsed.
  const renderOrgMenu = (side: 'bottom' | 'right') => (
    <DropdownMenuContent
      side={side}
      align="start"
      sideOffset={side === 'right' ? 8 : 4}
      className={cn('w-60', SIDEBAR_MENU_CONTENT)}
    >
      {/* The organisation you are in. */}
      <div className="flex items-center gap-2 rounded-lg bg-foreground/[0.05] p-1.5">
        <WorkspaceMark className="size-8 rounded-md" />
        <div className="flex min-w-0 flex-col gap-px leading-4 tracking-[-0.15px]">
          <span
            className="truncate text-[13px] text-foreground"
            title={organizationData?.tradeName ?? ''}
          >
            {organizationData?.tradeName ?? ''}
          </span>
          <span className="text-xs text-foreground/60">{mapRole(orgResponse?.role)}</span>
        </div>
      </div>
      {meData?.organizations
        ?.filter(org => org.slug !== organizationData?.slug)
        .map(org => (
          <DropdownMenuItem
            key={org.id}
            className={SIDEBAR_MENU_ITEM}
            onClick={() => {
              window.location.href = `/${org.slug}`
            }}
          >
            <WorkspaceMark />
            <span className="truncate">{org.tradeName}</span>
          </DropdownMenuItem>
        ))}
      {!isExpress && getInstanceQuery?.data?.multiOrganizationEnabled && (
        <DropdownMenuItem
          className={SIDEBAR_MENU_ITEM}
          onClick={() => navigate('/onboarding/organization')}
        >
          <Plus size={16} strokeWidth={1.5} />
          New organisation
        </DropdownMenuItem>
      )}
      <DropdownMenuSeparator className={SIDEBAR_MENU_SEPARATOR} />
      <DropdownMenuItem
        className={SIDEBAR_MENU_ITEM}
        onClick={() => navigate(`${basePath}/settings`)}
      >
        Settings
      </DropdownMenuItem>
      <DropdownMenuItem
        className={SIDEBAR_MENU_ITEM}
        // Switching keeps the menu open, so the change shows in place.
        onSelect={e => {
          e.preventDefault()
          toggleTheme()
        }}
      >
        Theme
        <span className="ml-auto text-xs text-foreground/40">{isDarkMode ? 'Dark' : 'Light'}</span>
      </DropdownMenuItem>
      {isDarkMode && (
        <DropdownMenuItem
          className={SIDEBAR_MENU_ITEM}
          // Experimental dark themes, in turn; keeps the menu open, like Theme.
          onSelect={e => {
            e.preventDefault()
            setDarkVariant(
              DARK_VARIANTS[(DARK_VARIANTS.indexOf(darkVariant) + 1) % DARK_VARIANTS.length]
            )
          }}
        >
          Dark style
          <span className="ml-auto text-xs text-foreground/40">
            {DARK_VARIANT_LABELS[darkVariant]}
          </span>
        </DropdownMenuItem>
      )}
      {!isExpress && guidePrefs.hidden && (
        <DropdownMenuItem className={SIDEBAR_MENU_ITEM} onClick={() => guidePrefs.setHidden(false)}>
          Show launch pad
        </DropdownMenuItem>
      )}
      <DropdownMenuSeparator className={SIDEBAR_MENU_SEPARATOR} />
      <DropdownMenuItem
        className={SIDEBAR_MENU_ITEM}
        onClick={() => logout('User clicked on logout')}
      >
        Log out
      </DropdownMenuItem>
    </DropdownMenuContent>
  )

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
      <EnvironmentBanner environment={tenant?.environment} />
      <Sidebar
        collapsible="icon"
        className="border-sidebar-border !top-(--env-banner-height) !h-auto [@media(max-height:900px)]:[&_[data-sidebar=menu-button]]:h-7 [@media(max-height:900px)]:[&_[data-sidebar=menu-button]]:py-1 [@media(max-height:900px)]:[&_[data-sidebar=menu-button]]:text-[13px]"
      >
        <SidebarHeader className="px-3 pt-3 pb-0">
          <SidebarMenu>
            <SidebarMenuItem>
              {isCollapsed ? (
                <Flex direction="column" align="center" className="gap-1.5">
                  <DropdownMenu>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <DropdownMenuTrigger asChild>
                          <button
                            type="button"
                            aria-label={organizationData?.tradeName ?? 'Organization'}
                            className="flex size-8 cursor-pointer items-center justify-center rounded-[5px] outline-none transition-colors hover:bg-sidebar-accent focus-visible:ring-2 focus-visible:ring-sidebar-ring data-[state=open]:bg-sidebar-accent"
                          >
                            <WorkspaceMark />
                          </button>
                        </DropdownMenuTrigger>
                      </TooltipTrigger>
                      <TooltipContent side="right">
                        {organizationData?.tradeName ?? ''}
                      </TooltipContent>
                    </Tooltip>
                    {renderOrgMenu('right')}
                  </DropdownMenu>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        onClick={toggleSidebar}
                        aria-label="Expand sidebar"
                        className="flex size-8 items-center justify-center rounded-[5px] text-faint-foreground hover:bg-sidebar-accent hover:text-foreground"
                      >
                        <SidebarPanelIcon />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent side="right">Expand sidebar</TooltipContent>
                  </Tooltip>
                </Flex>
              ) : (
                <Flex align="center" className="gap-2 pl-1 pr-1.5">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <SidebarMenuButton
                        asChild
                        className="cursor-pointer w-auto min-w-0 h-7 -ml-1 pl-1 pr-1.5"
                      >
                        <Flex align="center" className="gap-2">
                          <WorkspaceMark />
                          <span
                            className="min-w-0 truncate tracking-[-0.1px] text-sidebar-accent-foreground/90"
                            title={organizationData?.tradeName ?? ''}
                          >
                            {organizationData?.tradeName ?? ''}
                          </span>
                          <ChevronsUpDownIcon className="text-faint-foreground" />
                        </Flex>
                      </SidebarMenuButton>
                    </DropdownMenuTrigger>
                    {renderOrgMenu('bottom')}
                  </DropdownMenu>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button
                        type="button"
                        onClick={toggleSidebar}
                        aria-label="Collapse sidebar"
                        className="-mr-1.5 ml-auto flex size-7 shrink-0 items-center justify-center rounded-md text-faint-foreground hover:bg-sidebar-accent hover:text-foreground"
                      >
                        <SidebarPanelIcon />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent side="bottom">Collapse sidebar</TooltipContent>
                  </Tooltip>
                </Flex>
              )}
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarHeader>
        <SidebarContent className="gap-3 pt-3 [&>[data-sidebar=group]]:px-3 [&>[data-sidebar=group]]:py-0">
          {!isExpress && (
            <SidebarGroup>
              <SidebarMenu>
                <NewMenu />
              </SidebarMenu>
            </SidebarGroup>
          )}
          {!isExpress && <Slot name="layout.sidebarActions" />}
          <NavMain items={filteredSidebarItems.mainNav} />
        </SidebarContent>
        <SidebarFooter className="gap-2 p-3">
          {/* "What's new" (UpdatesPopover) is hidden for now; the tenant selector takes its place. */}
          {!isExpress && <TenantDropdown />}
          <div className="flex h-7 items-center justify-between pl-1 pr-2 group-data-[collapsible=icon]:h-auto group-data-[collapsible=icon]:flex-col-reverse group-data-[collapsible=icon]:gap-3 group-data-[collapsible=icon]:p-0">
            <Tooltip>
              <TooltipTrigger asChild>
                <span className="flex items-center justify-center">
                  <InitialAvatar
                    name={meData?.user?.firstName || meData?.user?.email}
                    shape="circle"
                  />
                </span>
              </TooltipTrigger>
              <TooltipContent side={isCollapsed ? 'right' : 'top'}>
                {[meData?.user?.firstName, meData?.user?.email].filter(Boolean).join(' · ')}
              </TooltipContent>
            </Tooltip>
            <div className="flex items-center gap-2 group-data-[collapsible=icon]:flex-col">
              {footerItems.map(renderFooterItem)}
            </div>
          </div>
        </SidebarFooter>
      </Sidebar>
      <PageIconContext.Provider value={pageIcon}>
        {/* Held to the window's height on a list page, whose table scrolls inside its card. */}
        <SidebarInset className="relative mt-(--env-banner-height) has-[[data-fit-viewport]]:h-[calc(100svh-var(--env-banner-height))] has-[[data-fit-viewport]]:min-h-0 has-[[data-fit-viewport]]:overflow-hidden">
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
          <SeedingBanner tenantId={tenant?.id} environment={tenant?.environment} />
          <div className="pointer-events-none absolute top-0 left-0 right-0 z-30 flex flex-col">
            <Slot name="layout.banners" />
            <TenantStatusBanner tenantId={tenant?.id} status={tenant?.status} />
          </div>
          <OverlayOutlet />
          {!isExpress && <LaunchPad />}
        </SidebarInset>
      </PageIconContext.Provider>
    </>
  )
}
