import type { To } from 'react-router-dom'

export interface ItemProps {
  label: string
  to: To
  end?: boolean
}
