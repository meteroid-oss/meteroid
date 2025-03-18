import { Badge, cn } from '@md/ui'
import { NavLink } from 'react-router-dom'

import type { FunctionComponent, ReactNode } from 'react'
import type { To } from 'react-router-dom'

export interface ItemProps {
  label: string | ReactNode
  to: To
  end?: boolean
  disabled?: boolean
  soon?: boolean
}

const ItemLink = ({
  isActive,
  children,
  disabled,
  soon = false,
}: {
  isActive: boolean
  children: ReactNode
  disabled?: boolean
  soon?: boolean
}) => {
  return (
    <span
      className={cn(
        'flex w-full justify-between items-center',
        'text-sm font-medium text-foreground leading-none',
        'rounded-md',
        'py-2 px-2.5',
        'transition-colors duration-200 ease-in-out',
        isActive ? 'bg-accent' : 'hover:bg-accent',
        disabled && 'text-muted-foreground'
      )}
    >
      <span>{children}</span>

      {soon && (
        <Badge variant="brand" className="text-xs ">
          Soon
        </Badge>
      )}
    </span>
  )
}

const Item: FunctionComponent<ItemProps> = ({ label, to, end, disabled, soon = false }) => {
  return (
    <li className={cn('block w-full', disabled && 'pointer-events-none')}>
      <NavLink to={to} end={end} viewTransition>
        {({ isActive }) => (
          <ItemLink isActive={isActive} disabled={disabled} soon={soon}>
            {label}
          </ItemLink>
        )}
      </NavLink>
    </li>
  )
}

export default Item
