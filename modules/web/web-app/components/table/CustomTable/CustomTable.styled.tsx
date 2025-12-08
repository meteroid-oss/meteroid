import type { FunctionComponent } from 'react'

const DefaultIndicatorIcon: FunctionComponent = () => (
  <svg width="6" height="10" viewBox="0 0 6 10" fill="none" xmlns="http://www.w3.org/2000/svg">
    <path d="M3 4.29296e-08L0 4L6 4L3 4.29296e-08Z" fill="currentColor" />
    <path d="M3 10L6 6L0 6L3 10Z" fill="currentColor" />
  </svg>
)

export const SortableDefaultIndicator: FunctionComponent = () => (
  <DefaultIndicatorIcon />
)

export const SortableIndicatorContainer: FunctionComponent<{ children: React.ReactNode }> = ({
  children,
}) => <div className="w-3.5 flex justify-center">{children}</div>
