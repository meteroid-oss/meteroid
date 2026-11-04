import {
  BetaBadge,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  Flex,
  SidebarGroup,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  cn,
  useSidebar,
  NewBadge,
} from '@ui/index'
import { ChevronRight, type LucideIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { NavLink, To, useLocation } from 'react-router-dom'

function Tree({
  item,
}: {
  item: {
    title: string
    url?: To
    icon: LucideIcon
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
  const { toggleSidebar, state } = useSidebar()
  const [isOpen, setIsOpen] = useState(
    item.url
      ? pathname.includes((item.url as string) ?? '')
      : item.items?.some(subItem => pathname.includes((subItem.url as string) ?? ''))
  )

  const isHome = item.title === 'Home'

  // Check if current path is a home path (contains only one slash after the base)
  const isHomeActive = isHome && /^\/[^/]+\/[^/]+$/.test(pathname)

  const isActive = isHome
    ? isHomeActive
    : item.url
      ? pathname.includes((item.url as string) ?? '')
      : false
  const isSubActive = item.items?.some(subItem => pathname.includes((subItem.url as string) ?? ''))

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

  if (item.items && item.items.length > 0) {
    return (
      <SidebarMenuItem>
        <Collapsible open={isOpen} onOpenChange={setIsOpen}>
          <CollapsibleTrigger asChild className="group/collapsible cursor-pointer">
            <SidebarMenuButton
              asChild
              tooltip={item.title}
              onClick={handCollapsibleClick}
              isActive={(isActive || isSubActive) && state === 'collapsed'}
            >
              <Flex align="center" justify="between">
                <Flex align="center" className="gap-3">
                  <item.icon size={16} className="min-w-4 min-h-4" />
                  <span>{item.title}</span>
                  {item.new && <NewBadge />}
                  {!isOpen && item.items.some(i => (i.count ?? 0) > 0) && (
                    <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-foreground/70" />
                  )}
                </Flex>
                <ChevronRight className="transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90" />
              </Flex>
            </SidebarMenuButton>
          </CollapsibleTrigger>
          <CollapsibleContent>
            {item.items.map((subItem, index) => {
              const isSubActive = pathname.includes((subItem.url as string) ?? '')

              return (
                <div
                  key={index}
                  className={cn(
                    'block w-full',
                    subItem.disabled && 'pointer-events-none opacity-60'
                  )}
                >
                  <NavLink to={subItem.url}>
                    <SidebarMenuButton key={index} isActive={isSubActive} className="pl-10">
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
          </CollapsibleContent>
        </Collapsible>
      </SidebarMenuItem>
    )
  }

  // If it's a regular item without sub-items
  const Icon = item.icon

  return (
    <SidebarMenuItem>
      <div className={cn('block w-full', item.disabled && 'pointer-events-none opacity-60')}>
        <NavLink to={item.url ?? ''}>
          <SidebarMenuButton isActive={isActive}>
            {Icon && <Icon className="mr-1 h-4 w-4" />}
            {item.title}
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
    icon: LucideIcon
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
