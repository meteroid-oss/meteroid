'use client'

import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { subDays, subYears } from 'date-fns'

export type DateRangePreset = 'last7days' | 'last30days' | 'last90days' | 'year' | 'allTime'

export interface DateRange {
  from: Date
  to: Date
}

export interface DatePresetSelectProps {
  value: DateRangePreset
  onChange: (preset: DateRangePreset) => void
  disabled?: boolean
}

const presetLabels: Record<DateRangePreset, string> = {
  last7days: 'Last 7 days',
  last30days: 'Last 30 days',
  last90days: 'Last 90 days',
  year: 'Year',
  allTime: 'All time',
}

export function getDateRangeFromPreset(preset: DateRangePreset): DateRange {
  const now = new Date()
  const to = now

  switch (preset) {
    case 'last7days':
      return { from: subDays(now, 7), to }
    case 'last30days':
      return { from: subDays(now, 30), to }
    case 'last90days': {
      return { from: subDays(now, 90), to }
    }
    case 'year':
      return { from: subYears(now, 1), to }
    case 'allTime':
      return { from: new Date(2020, 0, 1), to }
  }
}

export function DatePresetSelect({ value, onChange, disabled }: DatePresetSelectProps) {
  return (
    <Select value={value} onValueChange={v => onChange(v as DateRangePreset)} disabled={disabled}>
      <SelectTrigger className="w-[150px]">
        <SelectValue placeholder="Select range" />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="last7days">{presetLabels.last7days}</SelectItem>
        <SelectItem value="last30days">{presetLabels.last30days}</SelectItem>
        <SelectItem value="last90days">{presetLabels.last90days}</SelectItem>
        <SelectItem value="year">{presetLabels.year}</SelectItem>
        <SelectItem value="allTime">{presetLabels.allTime}</SelectItem>
      </SelectContent>
    </Select>
  )
}
