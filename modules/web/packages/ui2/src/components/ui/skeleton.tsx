import { cn } from '@ui2/lib/utils'

interface SkeletonProps {
  width?: number | string
  height?: number | string
}
function Skeleton({
  className,
  width,
  height,
  ...props
}: React.HTMLAttributes<HTMLDivElement> & SkeletonProps) {
  return (
    <div
      style={{ width, height }}
      className={cn('animate-pulse rounded-md bg-accent/50', className)}
      {...props}
    />
  )
}

export { Skeleton }
