import { cn } from '@md/ui'

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
        <div className="flex flex-col text-sm text-muted-foreground">
          <span>{label}</span>
        </div>
      </label>
      <div className={cn('mt-1 text-slate-1200 text-sm', childrenClassNames)}>{children}</div>
    </div>
  )
}
