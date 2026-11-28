const WIDTH = 96

const polyline = (values: number[], min: number, span: number, length: number, h: number) =>
  values
    .map((v, i) => {
      const x = (i / Math.max(length - 1, 1)) * WIDTH
      const y = h - ((v - min) / span) * (h - 2) - 1
      return `${x.toFixed(1)},${y.toFixed(1)}`
    })
    .join(' ')

/**
 * Dependency-free inline sparkline that fills its width and draws in `currentColor`. A `compare`
 * series shares the scale and the x positions, dashed and faint behind it.
 */
export const Sparkline = ({
  values,
  compare,
  height = 22,
}: {
  values: number[]
  compare?: number[]
  height?: number
}) => {
  const all = compare ? [...values, ...compare] : values
  const length = Math.max(values.length, compare?.length ?? 0)
  if (length < 2) {
    return <div style={{ height }} />
  }
  const min = Math.min(...all)
  const span = Math.max(...all) - min || 1
  return (
    <svg
      width="100%"
      height={height}
      viewBox={`0 0 ${WIDTH} ${height}`}
      preserveAspectRatio="none"
      aria-hidden
    >
      {compare && compare.length > 1 && (
        <polyline
          points={polyline(compare, min, span, length, height)}
          fill="none"
          className="stroke-foreground/25"
          strokeWidth="1.25"
          strokeDasharray="2 2"
          vectorEffect="non-scaling-stroke"
        />
      )}
      {values.length > 1 && (
        <polyline
          points={polyline(values, min, span, length, height)}
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinejoin="round"
          strokeLinecap="round"
          vectorEffect="non-scaling-stroke"
        />
      )}
    </svg>
  )
}
