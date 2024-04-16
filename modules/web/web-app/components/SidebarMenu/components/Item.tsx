import { cn } from '@md/ui'
import { NavLink } from 'react-router-dom'

import type { FunctionComponent, ReactNode } from 'react'
import type { To } from 'react-router-dom'

export interface ItemProps {
  label: string | ReactNode
  to: To
  end?: boolean
  disabled?: boolean
}

const ItemLink = ({
  isActive,
  children,
  disabled,
}: {
  isActive: boolean
  children: ReactNode
  disabled?: boolean
}) => {
  return (
    <span
      className={cn(
        'block w-full',
        'text-sm font-medium text-foreground leading-none',
        'rounded-md',
        'py-2 px-2.5',
        'transition-colors duration-200 ease-in-out',
        isActive ? 'bg-accent' : 'hover:bg-accent',
        disabled && 'text-muted-foreground'
      )}
    >
      {children}
    </span>
  )
}

const Item: FunctionComponent<ItemProps> = ({ label, to, end, disabled }) => {
  return (
    <li className={cn('block w-full', disabled && 'pointer-events-none')}>
      <NavLink to={to} end={end} unstable_viewTransition>
        {({ isActive }) => (
          <ItemLink isActive={isActive} disabled={disabled}>
            {label}
          </ItemLink>
        )}
      </NavLink>
    </li>
  )
}

export default Item
