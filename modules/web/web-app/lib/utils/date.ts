export const parseAndFormatDate = (dateString: string) => {
  if (!dateString) return 'N/A'
  return new Date(dateString).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

export const parseAndFormatDateOptional = (date?: string) => {
  return date ? parseAndFormatDate(date) : '-'
}

export const parseAndFormatDateTime = (dateString: string, precision: 'min' | 'sec' = 'min') => {
  if (!dateString) return 'N/A'
  return new Date(dateString).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    minute: 'numeric',
    hour: 'numeric',
    hour12: false,
    second: precision === 'sec' ? 'numeric' : undefined,
  })
}
