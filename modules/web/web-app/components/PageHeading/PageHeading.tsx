import type { FunctionComponent, ReactNode } from 'react'

interface PageHeadingProps {
  children: ReactNode
  count?: number
}

const PageHeading: FunctionComponent<PageHeadingProps> = ({ children, count }) => {
  return (
    <h1 className="text-2xl font-bold leading-none">
      {children}
      {count !== undefined && count >= 0 && (
        <span className="inline-block text-lg font-medium leading-none text-gray-500 ml-3">
          ({count})
        </span>
      )}
    </h1>
  )
}

export default PageHeading
