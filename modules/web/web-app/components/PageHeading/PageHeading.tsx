import type { FunctionComponent, ReactNode } from 'react'

interface PageHeadingProps {
  children: ReactNode
  count?: number
}

const PageHeading: FunctionComponent<PageHeadingProps> = ({ children, count }) => {
  return (
    <h1 className="text-2xl font-bold">
      {children}
      {count !== undefined && count >= 0 && (
        <span className="text-xs font-medium text-muted-foreground ml-1.5">({count})</span>
      )}
    </h1>
  )
}

export default PageHeading
