import { Menu } from '@md/ui'
import { FC } from 'react'

import { ProductMenuGroup, ProductMenuGroupItem } from './ProductMenu.types'
import ProductMenuItem from './ProductMenuItem'

interface ProductMenuProps {
  menu: ProductMenuGroup[]
}

export const ProductMenu: FC<ProductMenuProps> = ({ menu }) => {
  return (
    <div className="flex flex-col space-y-8 overflow-y-auto">
      <Menu type="pills">
        {menu.map((group: ProductMenuGroup, idx: number) => (
          <div key={group.title}>
            <div className="my-6 space-y-8">
              <div className="mx-3">
                <Menu.Group
                  title={
                    group.title ? (
                      <div className="flex flex-col space-y-2">
                        <span>{group.title}</span>
                      </div>
                    ) : null
                  }
                />
                <div>
                  {group.items.map((item: ProductMenuGroupItem) => (
                    <ProductMenuItem
                      key={item.to.toString()}
                      icon={item.icon}
                      label={item.label}
                      to={item.to}
                      end={item.end}
                    />
                  ))}
                </div>
              </div>
            </div>
            {idx !== menu.length - 1 && <div className="h-px w-full bg-scale-500"></div>}
          </div>
        ))}
      </Menu>
    </div>
  )
}
