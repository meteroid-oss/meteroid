import { FunctionComponent } from 'react'

interface BatchProgressBarProps {
  processed: number
  failed: number
  total: number
}

export const BatchProgressBar: FunctionComponent<BatchProgressBarProps> = ({
  processed,
  failed,
  total,
}) => {
  if (total === 0) return null
  const successPct = (processed / total) * 100
  const failPct = (failed / total) * 100

  return (
    <div className="relative h-2 w-full overflow-hidden rounded-full bg-primary/20">
      <div
        className="absolute inset-y-0 left-0 bg-primary transition-all"
        style={{ width: `${successPct}%` }}
      />
      <div
        className="absolute inset-y-0 bg-destructive transition-all"
        style={{ left: `${successPct}%`, width: `${failPct}%` }}
      />
    </div>
  )
}
