import { cn } from '@ui2/lib/utils'

interface SkeletonProps {
  width?: number
  height?: number
}
function Skeleton({
  className,
  width,
  height,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & SkeletonProps) {
  return (
    <div
      // style={{ width, height }}
      className={cn('animate-pulse rounded-md bg-accent bg-opacity-10', className)}
      {...props}
    />
  )
}

export { Skeleton }
