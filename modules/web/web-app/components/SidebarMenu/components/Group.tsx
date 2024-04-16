import Item, { ItemProps } from './Item'

import type { FunctionComponent } from 'react'

export interface GroupProps {
  label: string
  items: ItemProps[]
}

const Group: FunctionComponent<GroupProps> = ({ label, items }) => {
  return (
    <div className="mt-4">
      <span className="block text-xs font-semibold uppercase text-muted-foreground mb-1.5 pointer-events-none">
        {label}
      </span>
      <ul className="list-none flex flex-col gap-1">
        {items.map((item, index) => (
          <Item key={index} {...item} />
        ))}
      </ul>
    </div>
  )
}

export default Group
