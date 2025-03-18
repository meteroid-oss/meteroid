import { Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'

import { onClick } from './Item.hooks'
import { ItemLink } from './Item.styled'
import { NavigationItemType } from './Item.types'

import type { FunctionComponent } from 'react'

const Item: FunctionComponent<NavigationItemType> = ({ to, end, icon, label }) => {
  return (
    <li className="w-full">
      <Tooltip delayDuration={0}>
        <TooltipTrigger style={{ width: '100%' }}>
          <ItemLink to={to} end={end} onClick={onClick} viewTransition>
            {icon}
          </ItemLink>
        </TooltipTrigger>
        <TooltipContent side="right">{label}</TooltipContent>
      </Tooltip>
    </li>
  )
}

export default Item
