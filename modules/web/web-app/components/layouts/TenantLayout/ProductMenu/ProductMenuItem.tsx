import { Menu } from '@md/ui'
import { NavLink, To } from 'react-router-dom'

interface ProductMenuItemProps {
  to: To
  label: string
  icon?: JSX.Element
  end?: boolean
}

const ProductMenuItem = ({ icon, label, to, end }: ProductMenuItemProps) => {
  const menuItem = (
    <NavLink to={to} end={end}>
      {({ isActive }) => (
        <Menu.Item icon={icon} active={isActive}>
          <div className="flex w-full items-center justify-between gap-1">
            <div className="flex items-center gap-2 truncate w-full ">
              <span className="truncate">{label}</span>
            </div>
          </div>
        </Menu.Item>
      )}
    </NavLink>
  )

  return menuItem
}

export default ProductMenuItem
