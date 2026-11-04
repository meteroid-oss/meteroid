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
      <div className={cn('mt-1 text-foreground text-sm', childrenClassNames)}>{children}</div>
    </div>
  )
}

export const PriceComponentProperties: React.FC<{
  items: { label: string; value: React.ReactNode }[]
  className?: string
}> = ({ items, className }) => {
  if (items.length === 0) return null
  return (
    <div className={cn('flex flex-wrap gap-x-10 gap-y-3', className)}>
      {items.map(item => (
        <PriceComponentProperty key={item.label} label={item.label}>
          {item.value}
        </PriceComponentProperty>
      ))}
    </div>
  )
}
