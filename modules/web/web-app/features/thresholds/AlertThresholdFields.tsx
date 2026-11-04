import { Input, Label } from '@md/ui'

import { parseThresholds } from './thresholds'

const preview = (values: string, limit: string): string => {
  const numericLimit = Number(limit)
  if (!limit.trim() || !Number.isFinite(numericLimit)) return ''
  try {
    return parseThresholds(values)
      .map(pct => `${pct}% of ${limit.trim()} = ${Number(((numericLimit * pct) / 100).toFixed(9))}`)
      .join('; ')
  } catch {
    return ''
  }
}

export const AlertThresholdFields = ({
  values,
  limit,
  error,
  onChange,
}: {
  values: string
  limit: string
  error?: string
  onChange: (values: string) => void
}) => {
  const example = values.trim() ? preview(values, limit) : ''
  return (
    <div className="space-y-2">
      <Label>Alert thresholds (optional)</Label>
      <Input
        aria-label="Alert thresholds"
        aria-invalid={Boolean(error)}
        value={values}
        onChange={event => onChange(event.target.value)}
        placeholder="e.g. 50, 80, 100"
      />
      <p className="text-xs text-muted-foreground">
        Percentages, separated by commas. Reaching 100% always alerts.
      </p>
      {example && <p className="text-xs text-muted-foreground">{example}</p>}
      {error && <p className="text-xs font-medium text-destructive">{error}</p>}
    </div>
  )
}
