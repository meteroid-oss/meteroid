// Leaf-agnostic editable model for rule builders (grouped ALL/ANY + per-condition
// negate). A builder supplies its own leaf union `L`; the boolean envelope
// (`and` / `or` / `not`) is shared and serde-compatible with the Rust variants.

export type BoolPredicate<L> =
  | { and: { children: RulePredicate<L>[] } }
  | { or: { children: RulePredicate<L>[] } }
  | { not: { child: RulePredicate<L> } }

export type RulePredicate<L> = L | BoolPredicate<L>

export const isLeaf = <L>(p: RulePredicate<L>): p is L =>
  !('and' in (p as object)) && !('or' in (p as object)) && !('not' in (p as object))

export type Cond<L> = { negate: boolean; leaf: L }
export type Group<L> = { negate: boolean; op: 'and' | 'or'; conds: Cond<L>[] }
export type Row<L> = { t: 'cond'; cond: Cond<L> } | { t: 'group'; group: Group<L> }
export type Model<L> = { op: 'and' | 'or'; rows: Row<L>[] }

const serCond = <L>(c: Cond<L>): RulePredicate<L> =>
  c.negate ? { not: { child: c.leaf } } : c.leaf
const serGroup = <L>(g: Group<L>): RulePredicate<L> => {
  const base: RulePredicate<L> =
    g.op === 'and'
      ? { and: { children: g.conds.map(serCond) } }
      : { or: { children: g.conds.map(serCond) } }
  return g.negate ? { not: { child: base } } : base
}
const serRow = <L>(r: Row<L>): RulePredicate<L> =>
  r.t === 'cond' ? serCond(r.cond) : serGroup(r.group)
// A root ANY becomes a single OR predicate; a root ALL is the flat predicate list
// (the server ANDs the root list), so existing flat definitions round-trip unchanged.
export const serializeModel = <L>(m: Model<L>): RulePredicate<L>[] =>
  m.op === 'or' ? [{ or: { children: m.rows.map(serRow) } }] : m.rows.map(serRow)

const groupOf = <L>(
  p: RulePredicate<L>
): { op: 'and' | 'or'; children: RulePredicate<L>[] } | null =>
  'and' in (p as object)
    ? { op: 'and', children: (p as { and: { children: RulePredicate<L>[] } }).and.children }
    : 'or' in (p as object)
      ? { op: 'or', children: (p as { or: { children: RulePredicate<L>[] } }).or.children }
      : null
const notChild = <L>(p: RulePredicate<L>): RulePredicate<L> | null =>
  'not' in (p as object) ? (p as { not: { child: RulePredicate<L> } }).not.child : null

const parseCond = <L>(p: RulePredicate<L>): Cond<L> => {
  const child = notChild(p)
  return child !== null && isLeaf(child)
    ? { negate: true, leaf: child }
    : { negate: false, leaf: isLeaf(p) ? p : (p as unknown as L) }
}
const parseRow = <L>(p: RulePredicate<L>): Row<L> => {
  if (isLeaf(p)) return { t: 'cond', cond: { negate: false, leaf: p } }
  const child = notChild(p)
  if (child !== null) {
    if (isLeaf(child)) return { t: 'cond', cond: { negate: true, leaf: child } }
    const g = groupOf(child)
    if (g) return { t: 'group', group: { negate: true, op: g.op, conds: g.children.map(parseCond) } }
    return { t: 'cond', cond: { negate: true, leaf: child as unknown as L } }
  }
  const g = groupOf(p)
  if (g) return { t: 'group', group: { negate: false, op: g.op, conds: g.children.map(parseCond) } }
  return { t: 'cond', cond: { negate: false, leaf: p as unknown as L } }
}
export const parseModel = <L>(predicates: RulePredicate<L>[]): Model<L> => {
  const only = predicates.length === 1 ? groupOf(predicates[0]) : null
  return only?.op === 'or'
    ? { op: 'or', rows: only.children.map(parseRow) }
    : { op: 'and', rows: predicates.map(parseRow) }
}

const sanitizeOne = <L>(p: RulePredicate<L>): RulePredicate<L> | null => {
  const g = groupOf(p)
  if (g) {
    const children = sanitizePredicates(g.children)
    if (children.length === 0) return null // drop an empty group (server rejects it)
    if (children.length === 1) return children[0] // a single-child group is redundant
    return g.op === 'and' ? { and: { children } } : { or: { children } }
  }
  const child = notChild(p)
  if (child !== null) {
    const clean = sanitizeOne(child)
    return clean ? { not: { child: clean } } : null
  }
  return p
}

/**
 * Strip empty/degenerate boolean groups from a definition so it never contains a
 * group with no children (which the server rejects). Returns the pruned predicate
 * list; an empty result means there is no usable condition.
 */
export const sanitizePredicates = <L>(predicates: RulePredicate<L>[]): RulePredicate<L>[] =>
  predicates.map(sanitizeOne).filter((p): p is RulePredicate<L> => p !== null)
