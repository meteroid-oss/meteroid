import { Fragment, FunctionComponent } from 'react'

import { NAVIGATION_ITEMS } from './Items.data'
import Item from './components/Item'

const Items: FunctionComponent = () => {
  return (
    <ul className="max-w-[55px] w-full flex flex-col gap-2">
      {NAVIGATION_ITEMS.map(item => (
        <Fragment key={item.label}>
          <Item {...item} />
          {item.divider && <hr className="nav-item-divider" />}
        </Fragment>
      ))}
    </ul>
  )
}

export default Items
