const pctOf = (value: bigint, base: bigint) => {
  if (value <= 0n) return 0
  if (base <= 0n) return 100
  return Math.min(100, Number((value * 10_000n) / base) / 100)
}

/** Used credit as a solid segment, the projection to period end as a hatched one after it. */
export const CreditBar = ({
  base,
  used,
  projected,
  label,
}: {
  base: bigint
  used: bigint
  projected?: bigint
  label: string
}) => {
  const usedPct = pctOf(used, base)
  const projectedPct = projected !== undefined ? Math.max(usedPct, pctOf(projected, base)) : usedPct
  return (
    <div
      role="img"
      aria-label={label}
      style={{
        position: 'relative',
        height: 10,
        borderRadius: 999,
        background: 'var(--mtp-track)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          position: 'absolute',
          left: 0,
          top: 0,
          bottom: 0,
          width: `${usedPct}%`,
          borderRadius: 999,
          background: 'var(--mtp-fill)',
        }}
      />
      {projectedPct > usedPct && (
        <div
          className="mtp-hatch"
          style={{
            position: 'absolute',
            left: `${usedPct}%`,
            top: 0,
            bottom: 0,
            width: `${projectedPct - usedPct}%`,
          }}
        />
      )}
    </div>
  )
}
