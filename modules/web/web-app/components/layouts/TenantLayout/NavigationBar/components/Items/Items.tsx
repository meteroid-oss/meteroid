import { Fragment, FunctionComponent } from 'react'

import { NAVIGATION_ITEMS } from './Items.data'
import { ItemDivider, StyledItems } from './Items.styled'
import Item from './components/Item'

const Items: FunctionComponent = () => {
  return (
    <StyledItems>
      {NAVIGATION_ITEMS.map(item => (
        <Fragment key={item.label}>
          <Item {...item} />
          {item.divider && <ItemDivider />}
        </Fragment>
      ))}
    </StyledItems>
  )
}

export default Items
