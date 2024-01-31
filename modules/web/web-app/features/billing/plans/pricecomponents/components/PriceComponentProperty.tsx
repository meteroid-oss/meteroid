import { cn } from '@ui/lib/cn'

export type Props = {
  className?: string
  label: React.ReactNode
  children?: React.ReactNode
  childrenClassNames?: string
}

export const PriceComponentProperty: React.FC<Props> = ({
  className,
  label,
  children,
  childrenClassNames,
}) => {
  return (
    <div className={className}>
      <label className="flex">
        <div className="flex flex-col text-sm text-slate-1100">
          <span>{label}</span>
        </div>
      </label>
      <div className={cn('mt-1 text-scale-1200 text-sm', childrenClassNames)}>{children}</div>
    </div>
  )
}
