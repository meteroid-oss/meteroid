import { menuItem, menuSurface } from '@ui/components/ui/menu-styles'
import {
  BetaBadge,
  Flex,
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
  SidebarGroup,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  cn,
  useSidebar,
  NewBadge,
} from '@ui/index'
import { type ComponentType, useEffect, useId, useState } from 'react'
import { NavLink, To, useLocation } from 'react-router'

import { ChevronDownIcon } from '@/components/icons/AppIcons'
import {
  SIDEBAR_MENU_CONTENT,
  SIDEBAR_MENU_ITEM,
  SIDEBAR_MENU_ITEM_CURRENT,
} from '@/components/layouts/shared/sidebarMenuStyles'

/** A nav item's icon; `filledClassName` (see AppIcons) gives it a filled look when selected. */
export type NavIcon = ComponentType<{ className?: string }> & { filledClassName?: string }

/** The item's icon in a 20px slot: quiet at rest, full contrast when hovered or selected. Icons
 * keep their own drawn size (≤16px); larger ones, e.g. lucide's 24px default, are scaled down. */
const NavItemIcon = ({
  icon: Icon,
  active,
  filled = active,
}: {
  icon: NavIcon
  active: boolean
  /** Filled look: the selected item, or a section holding the selected page. */
  filled?: boolean
}) => (
  <span
    className={cn(
      'flex size-5 shrink-0 items-center justify-center transition-colors [&>svg]:max-h-4 [&>svg]:max-w-4',
      active
        ? 'text-sidebar-accent-foreground'
        : 'text-sidebar-icon group-hover/nav:text-sidebar-accent-foreground'
    )}
  >
    <Icon className={filled ? Icon.filledClassName : undefined} />
  </span>
)

function Tree({
  item,
}: {
  item: {
    title: string
    url?: To
    icon: NavIcon
    disabled?: boolean
    preview?: boolean
    new?: boolean
    items?: {
      title: string
      url: To
      disabled?: boolean
      beta?: boolean
      preview?: boolean
      new?: boolean
      /** What waits on the caller there. */
      count?: number
    }[]
  }
}) {
  const { pathname } = useLocation()
  const { toggleSidebar, state, isMobile } = useSidebar()
  const [isOpen, setIsOpen] = useState(
    item.url
      ? pathname.includes((item.url as string) ?? '')
      : item.items?.some(subItem => pathname.includes((subItem.url as string) ?? ''))
  )

  const panelId = useId()

  const isHome = item.title === 'Home'

  // Check if current path is a home path (contains only one slash after the base)
  const isHomeActive = isHome && /^\/[^/]+\/[^/]+$/.test(pathname)

  const isActive = isHome
    ? isHomeActive
    : item.url
      ? pathname.includes((item.url as string) ?? '')
      : false
  const currentSubUrl = item.items
    ?.map(subItem => (subItem.url as string) ?? '')
    .filter(url => url && pathname.includes(url))
    .sort((a, b) => b.length - a.length)[0]
  const isSubActive = currentSubUrl !== undefined

  const handCollapsibleClick = () => {
    if (state === 'collapsed') {
      toggleSidebar()
    }
  }

  useEffect(() => {
    if (state === 'collapsed') {
      setIsOpen(false)
    }
  }, [state])

  // Collapsed: only the icon; its sub-items open beside it on hover (or focus), like a menu.
  if (item.items && item.items.length > 0 && state === 'collapsed' && !isMobile) {
    const active = isActive || !!isSubActive
    return (
      <SidebarMenuItem>
        <HoverCard openDelay={80} closeDelay={120}>
          <HoverCardTrigger asChild>
            <SidebarMenuButton
              isActive={active}
              onClick={toggleSidebar}
              aria-label={item.title}
              className="group/nav"
            >
              <NavItemIcon icon={item.icon} active={active} filled={active} />
            </SidebarMenuButton>
          </HoverCardTrigger>
          <HoverCardContent
            side="right"
            align="start"
            sideOffset={8}
            className={cn(menuSurface, SIDEBAR_MENU_CONTENT, 'w-52')}
          >
            <div className="px-2 pt-1.5 pb-1 text-xs font-medium text-muted-foreground">
              {item.title}
            </div>
            {item.items.map((subItem, index) => {
              const current = subItem.url === currentSubUrl
              return (
                <NavLink
                  key={index}
                  to={subItem.url}
                  className={cn(
                    'flex select-none items-center outline-none hover:text-foreground hover:before:opacity-100 hover:before:scale-100 focus-visible:text-foreground focus-visible:before:opacity-100 focus-visible:before:scale-100',
                    menuItem,
                    SIDEBAR_MENU_ITEM,
                    current && SIDEBAR_MENU_ITEM_CURRENT,
                    subItem.disabled && 'pointer-events-none opacity-60'
                  )}
                >
                  <span className="truncate">{subItem.title}</span>
                  {subItem.beta && <BetaBadge />}
                  {subItem.new && <NewBadge />}
                  {!!subItem.count && (
                    <span
                      className="ml-auto rounded-full bg-foreground/10 px-1.5 text-[10.5px] font-medium tabular-nums text-foreground"
                      aria-label={`${subItem.count} waiting on you`}
                    >
                      {subItem.count > 99 ? '99+' : subItem.count}
                    </span>
                  )}
                </NavLink>
              )
            })}
          </HoverCardContent>
        </HoverCard>
      </SidebarMenuItem>
    )
  }

  if (item.items && item.items.length > 0) {
    return (
      // data-open drives the fold (see .mt-nav-group in main.css). The panel stays mounted so it can
      // fold out; while closed, its links are hidden from Tab and screen readers.
      <SidebarMenuItem className="mt-nav-group" data-open={isOpen ? 'true' : 'false'}>
        <SidebarMenuButton
          type="button"
          tooltip={item.title}
          onClick={() => {
            handCollapsibleClick()
            setIsOpen(open => !open)
          }}
          aria-expanded={isOpen}
          aria-controls={panelId}
          isActive={(isActive || isSubActive) && state === 'collapsed'}
          className="group/nav cursor-pointer pr-1.5"
        >
          <Flex align="center" justify="between" className="w-full min-w-0">
            <Flex align="center" className="gap-1.5 min-w-0">
              <NavItemIcon
                icon={item.icon}
                active={(isActive || !!isSubActive) && state === 'collapsed'}
                filled={isActive || !!isSubActive}
              />
              <span className="truncate">{item.title}</span>
              {item.new && <NewBadge />}
              {!isOpen && item.items.some(i => (i.count ?? 0) > 0) && (
                <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-foreground/70" />
              )}
            </Flex>
            <span className="mt-nav-chevron group-data-[collapsible=icon]:hidden">
              <ChevronDownIcon className="text-faint-foreground" />
            </span>
          </Flex>
        </SidebarMenuButton>
        <div id={panelId} className="mt-nav-group-panel">
          <div className="mt-nav-group-inner">
            {item.items.map((subItem, index) => {
              const isSubActive = subItem.url === currentSubUrl

              return (
                <div
                  key={index}
                  className={cn(
                    'block w-full',
                    subItem.disabled && 'pointer-events-none opacity-60'
                  )}
                >
                  <NavLink to={subItem.url}>
                    <SidebarMenuButton key={index} isActive={isSubActive} className="pl-[32px]">
                      <Flex align="center" className="w-full gap-2">
                        <span>{subItem.title}</span>
                        {subItem.beta && <BetaBadge />}
                        {subItem.new && <NewBadge />}
                        {!!subItem.count && (
                          <span
                            className="ml-auto rounded-full bg-foreground/10 px-1.5 text-[10.5px] font-medium tabular-nums text-foreground"
                            aria-label={`${subItem.count} waiting on you`}
                          >
                            {subItem.count > 99 ? '99+' : subItem.count}
                          </span>
                        )}
                      </Flex>
                    </SidebarMenuButton>
                  </NavLink>
                </div>
              )
            })}
          </div>
        </div>
      </SidebarMenuItem>
    )
  }

  // If it's a regular item without sub-items
  const Icon = item.icon

  return (
    <SidebarMenuItem>
      <div className={cn('block w-full', item.disabled && 'pointer-events-none opacity-60')}>
        <NavLink to={item.url ?? ''}>
          <SidebarMenuButton isActive={isActive} tooltip={item.title} className="group/nav">
            {Icon && <NavItemIcon icon={Icon} active={isActive} />}
            <span>{item.title}</span>
          </SidebarMenuButton>
        </NavLink>
      </div>
    </SidebarMenuItem>
  )
}

export function NavMain({
  items,
}: {
  items: {
    title: string
    url?: To
    icon: NavIcon
    isActive?: boolean
    disabled?: boolean
    new?: boolean
    items?: {
      title: string
      url: To
      isActive?: boolean
      disabled?: boolean
      beta?: boolean
      new?: boolean
      count?: number
    }[]
  }[]
}) {
  return (
    <SidebarGroup>
      <SidebarMenu>
        {items.map((item, index) => (
          <Tree key={index} item={item} />
        ))}
      </SidebarMenu>
    </SidebarGroup>
  )
}
