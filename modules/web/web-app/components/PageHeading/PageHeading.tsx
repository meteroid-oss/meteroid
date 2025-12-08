import type { FunctionComponent, ReactNode } from 'react'

interface PageHeadingProps {
  children: ReactNode
  count?: number
}

const PageHeading: FunctionComponent<PageHeadingProps> = ({ children, count }) => {
  return (
    <h1 className="text-xl font-bold leading-none">
      {children}
      {count !== undefined && count >= 0 && (
        <span className="inline-block text-lg font-medium leading-none text-muted-foreground ml-1.5">
          ({count})
        </span>
      )}
    </h1>
  )
}

export default PageHeading
