import { ReactNode } from 'react'
import { Link } from 'react-router-dom'

interface Props {
  label: string
  value: ReactNode
  to?: string
}

export const Property = ({ to, value, label }: Props) => (
  <div className="flex flex-row">
    <div className="w-32 flex-none">
      <span className="text-sm text-slate-1000">{label}</span>
    </div>
    <div className="self-center">
      {to ? <Link to={to}>{value}</Link> : <span className="text-sm">{value}</span>}
    </div>
  </div>
)
