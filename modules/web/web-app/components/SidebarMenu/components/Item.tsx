import { NavLink } from 'react-router-dom'

import type { To } from 'react-router-dom'
import type { FunctionComponent, ReactNode } from 'react'
import { cn } from '@ui/lib'

export interface ItemProps {
  label: string | ReactNode
  to: To
  end?: boolean
}

const ItemLink = ({ isActive, children }: { isActive: boolean; children: ReactNode }) => {
  return (
    <span
      className={cn(
        'block w-full',
        'text-sm font-medium text-foreground leading-none',
        'rounded-md',
        'py-2 px-2.5',
        'transition-colors duration-200 ease-in-out',
        isActive ? 'bg-accent' : 'hover:bg-accent'
      )}
    >
      {children}
    </span>
  )
}

const Item: FunctionComponent<ItemProps> = ({ label, to, end }) => {
  return (
    <li className="block w-full">
      <NavLink to={to} end={end}>
        {({ isActive }) => <ItemLink isActive={isActive}>{label}</ItemLink>}
      </NavLink>
    </li>
  )
}

export default Item
