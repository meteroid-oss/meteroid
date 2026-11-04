/** "3d before" / "Due day" / "+7d after" — a reminder step's offset from the due date. */
export const offsetPill = (offset: number): string =>
  offset < 0 ? `${-offset}d before` : offset > 0 ? `+${offset}d after` : 'Due day'
