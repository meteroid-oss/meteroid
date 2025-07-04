import { cn } from '@ui/lib'

interface PageSectionProps {
  className?: string
  hidden?: boolean
  header?: {
    title: React.ReactNode
    subtitle?: string
    actions?: React.ReactNode
  }
  children: React.ReactNode
}
export const PageSection: React.FC<PageSectionProps> = ({
  children,
  header,
  className = '',
  hidden,
}) => {
  return (
    <div className={cn('relative pb-4 ml-[1px]', className, hidden && 'hidden')}>
      {header && (
        <div className="pb-3 border-b border-muted-foreground space-y-1">
          <div className="flex justify-between items-end">
            <h2 className="text-xl font-semibold">{header.title}</h2>
            <div>{header.actions}</div>
          </div>
          {header.subtitle && (
            <div className="text-muted-foreground text-sm">{header.subtitle}</div>
          )}
        </div>
      )}
      <div className="py-6  ">{children}</div>
    </div>
  )
}
