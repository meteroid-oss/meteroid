interface PageSectionProps {
  header?: {
    title: string
    subtitle?: string
    actions?: React.ReactNode
  }
  children: React.ReactNode
}
export const PageSection: React.FC<PageSectionProps> = ({ children, header }) => {
  return (
    <div className="relative pb-6">
      {header && (
        <div className="pb-4 border-b border-slate-600 space-y-1">
          <div className="flex justify-between">
            <h2 className="text-xl font-semibold">{header.title}</h2>
            <div>{header.actions}</div>
          </div>
          {header.subtitle && <div className="text-scale-900 text-sm">{header.subtitle}</div>}
        </div>
      )}
      <div>{children}</div>
    </div>
  )
}
