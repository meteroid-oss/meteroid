interface PageSectionProps {
  className?: string
  header?: {
    title: string
    subtitle?: string
    actions?: React.ReactNode
  }
  children: React.ReactNode
}
export const PageSection: React.FC<PageSectionProps> = ({ children, header, className = '' }) => {
  return (
    <div className={`relative pb-4 ${className}`}>
      {header && (
        <div className="pb-3 border-b border-slate-600 space-y-1">
          <div className="flex justify-between items-end">
            <h2 className="text-xl font-semibold">{header.title}</h2>
            <div>{header.actions}</div>
          </div>
          {header.subtitle && <div className="text-scale-900 text-sm">{header.subtitle}</div>}
        </div>
      )}
      <div className="py-6">{children}</div>
    </div>
  )
}
