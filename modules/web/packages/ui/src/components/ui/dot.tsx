/*
 <svg className={`text-warning h-2`} fill="currentColor" viewBox="0 0 8 8">
                        <circle cx="4" cy="4" r="3" />
                      </svg>
                      */

export const Dot = ({ className }: { className?: string }) => {
  return (
    <svg className={className} fill="currentColor" viewBox="0 0 8 8">
      <circle cx="4" cy="4" r="3" />
    </svg>
  )
}
