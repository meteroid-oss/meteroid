import { Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { NavLink } from 'react-router-dom'

import { onClick } from './Item.hooks'
import { NavigationItemType } from './Item.types'

import type { FunctionComponent } from 'react'

const Item: FunctionComponent<NavigationItemType> = ({ to, end, icon, label }) => {
  return (
    <li className="w-full">
      <Tooltip delayDuration={0}>
        <TooltipTrigger style={{ width: '100%' }}>
          <NavLink to={to} end={end} onClick={onClick} className="nav-item-link">
            {icon}
          </NavLink>
        </TooltipTrigger>
        <TooltipContent side="right">{label}</TooltipContent>
      </Tooltip>
    </li>
  )
}

export default Item
