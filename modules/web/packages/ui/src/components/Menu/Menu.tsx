import { cn } from '@ui/lib/cn'

import { twMenuClasses } from './Menu.styles'
import { MenuContextProvider, useMenuContext } from './MenuContext'

interface MenuProps {
  children: React.ReactNode
  className?: string
  ulClassName?: string
  style?: React.CSSProperties
  type?: 'text' | 'pills' | 'border'
}

function Menu({ children, className, ulClassName, style, type = 'text' }: MenuProps) {
  return (
    <nav
      role="menu"
      aria-label="Sidebar"
      aria-orientation="vertical"
      aria-labelledby="options-menu"
      className={className}
      style={style}
    >
      <MenuContextProvider type={type}>
        <ul className={ulClassName}>{children}</ul>
      </MenuContextProvider>
    </nav>
  )
}

interface ItemProps {
  children: React.ReactNode
  icon?: React.ReactNode
  active?: boolean
  onClick?: React.MouseEventHandler<HTMLAnchorElement>
  style?: React.CSSProperties
}

export function Item({ children, icon, active, onClick, style }: ItemProps) {
  const __styles = twMenuClasses.menu.item

  const { type } = useMenuContext()

  const classes = [
    __styles.base,
    __styles.variants[type].base,
    active ? __styles.variants[type].active : __styles.variants[type].normal,
  ]

  const contentClasses = [
    __styles.content.base,
    active ? __styles.content.active : __styles.content.normal,
  ]

  const iconClasses = [__styles.icon.base, active ? __styles.icon.active : __styles.icon.normal]

  const Item = onClick ? 'a' : 'span'

  return (
    <li role="menuitem" className="outline-none">
      <Item
        style={style}
        className={cn(classes)}
        onClick={onClick}
        aria-current={active ? 'page' : undefined}
      >
        {icon && <div className={`${cn(iconClasses)} min-w-fit`}>{icon}</div>}
        <span className={cn(contentClasses)}>{children}</span>
      </Item>
    </li>
  )
}

interface GroupProps {
  children?: React.ReactNode
  icon?: React.ReactNode
  title: string | React.ReactNode
}

export function Group({ children, icon, title }: GroupProps) {
  const __styles = twMenuClasses.menu.group

  const { type } = useMenuContext()
  return (
    <div className={[__styles.base, __styles.variants[type]].join(' ')}>
      {icon && <span className={__styles.icon}>{icon}</span>}
      <span className={__styles.content}>{title}</span>
      {children}
    </div>
  )
}

Menu.Item = Item
Menu.Group = Group
export default Menu
