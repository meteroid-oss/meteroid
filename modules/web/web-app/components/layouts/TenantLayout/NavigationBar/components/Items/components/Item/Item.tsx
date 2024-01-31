import { Tooltip, TooltipArrow, TooltipContent, TooltipPortal, TooltipTrigger } from '@md/ui'

import { onClick } from './Item.hooks'
import { ItemLink, StyledItem } from './Item.styled'
import { NavigationItemType } from './Item.types'

import type { FunctionComponent } from 'react'

const Item: FunctionComponent<NavigationItemType> = ({ to, end, icon, label }) => {
  return (
    <StyledItem>
      <Tooltip delayDuration={0}>
        <TooltipTrigger style={{ width: '100%' }}>
          <ItemLink to={to} end={end} onClick={onClick}>
            {icon}
          </ItemLink>
        </TooltipTrigger>
        <TooltipPortal>
          <TooltipContent side="right">
            <TooltipArrow />
            {label}
          </TooltipContent>
        </TooltipPortal>
      </Tooltip>
    </StyledItem>
  )
}

export default Item
