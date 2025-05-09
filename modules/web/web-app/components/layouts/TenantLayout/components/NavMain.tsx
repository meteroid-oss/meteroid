import {
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
    items?: {
      title: string
      url: To
      disabled?: boolean
    }[]
  }
}) {
  const { pathname } = useLocation()
  const { toggleSidebar, state } = useSidebar()
  const [isOpen, setIsOpen] = useState(
    item.url
      ? pathname.includes((item.url as string) ?? '')
      : false || item.items?.some(subItem => pathname.includes((subItem.url as string) ?? ''))
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
                <Flex align="center" className="gap-4">
                  <item.icon size={16} className="min-w-4 min-h-4" />
                  <span>{item.title}</span>
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
                  className={cn('block w-full', item.disabled && 'pointer-events-none')}
                >
                  <NavLink to={subItem.url} viewTransition>
                    <SidebarMenuButton key={index} isActive={isSubActive} className="pl-10">
                      {subItem.title}
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
      <div className={cn('block w-full', item.disabled && 'pointer-events-none')}>
        <NavLink to={item.url ?? ''} viewTransition>
          <SidebarMenuButton isActive={isActive}>
            {Icon && <Icon className="mr-2 h-4 w-4" />}
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
    items?: {
      title: string
      url: To
      isActive?: boolean
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
