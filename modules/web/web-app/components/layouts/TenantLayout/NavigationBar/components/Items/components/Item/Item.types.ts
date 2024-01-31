import type { To } from 'react-router-dom'

export type NavigationItemType = {
  label: string
  to: To
  end?: boolean
  icon: React.ReactNode
  divider?: boolean
}
