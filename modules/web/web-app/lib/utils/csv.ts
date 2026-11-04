// Shared client-side CSV builder for the bounded reports (small, already-loaded
// data — no server round-trip). RFC-4180 escaping: a field is quoted and its
// embedded quotes doubled only when it contains a quote, comma, or newline.
const escape = (v: string | number): string => {
  const s = String(v ?? '')
  return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
}

export const downloadCsv = (
  filename: string,
  header: string[],
  rows: (string | number)[][]
): void => {
  const body = [header, ...rows].map(r => r.map(escape).join(',')).join('\n')
  const blob = new Blob([body], { type: 'text/csv;charset=utf-8;' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  a.click()
  URL.revokeObjectURL(url)
}
