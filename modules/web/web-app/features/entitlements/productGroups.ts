export type ProductRefLike = { id: string; name: string }

export const TENANT_WIDE_LABEL = 'All products'

export type ProductGroup<T> = {
  /** The product's id; `''` for tenant-wide features. */
  key: string
  product?: ProductRefLike
  /** The product's name, or "All products"; a picker's heading. */
  name: string
  /** "<name> · <count>"; a list's header. */
  label: string
  items: T[]
}

type Options<T> = {
  /** Counts items of the same key once, e.g. the rows of one feature. */
  countBy?: (item: T) => string
}

const nameOf = (product: ProductRefLike | undefined) => (product ? product.name : TENANT_WIDE_LABEL)

export const productGroupLabel = (product: ProductRefLike | undefined, count: number) =>
  `${nameOf(product)} · ${count}`

const byName = (a: ProductRefLike, b: ProductRefLike) =>
  a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }) || a.id.localeCompare(b.id)

/** Products by name, tenant-wide features last; each group keeps the items' order. */
export const productGroups = <T>(
  items: readonly T[],
  productOf: (item: T) => ProductRefLike | undefined,
  { countBy }: Options<T> = {}
): ProductGroup<T>[] => {
  const groups = new Map<string, { product?: ProductRefLike; items: T[] }>()
  for (const item of items) {
    const product = productOf(item)
    const key = product?.id ?? ''
    const group = groups.get(key)
    if (group) group.items.push(item)
    else groups.set(key, { product, items: [item] })
  }
  return [...groups.entries()]
    .sort(([keyA, a], [keyB, b]) =>
      keyA && keyB ? byName(a.product!, b.product!) : Number(!keyA) - Number(!keyB)
    )
    .map(([key, { product, items }]) => ({
      key,
      product,
      name: nameOf(product),
      label: productGroupLabel(product, countBy ? new Set(items.map(countBy)).size : items.length),
      items,
    }))
}

/** The groups to head; undefined when every item shares a product (or has none): render flat. */
export const productSections = <T>(
  items: readonly T[],
  productOf: (item: T) => ProductRefLike | undefined,
  options?: Options<T>
): ProductGroup<T>[] | undefined => {
  const groups = productGroups(items, productOf, options)
  return groups.length > 1 ? groups : undefined
}

export type ProductRows<T> = {
  rows: T[]
  /** Unset when flat. */
  groupOf?: (row: T) => { key: string; label: string }
  /** The header to insert before `rows[index]`, where a group starts. */
  headerAt: (index: number) => string | undefined
}

/**
 * Rows in group order with the header of each, for tables that insert a header row where the
 * group changes. `force` heads a single group too, e.g. one page of a longer list.
 */
export const productRows = <T>(
  items: readonly T[],
  productOf: (item: T) => ProductRefLike | undefined,
  { force = false, ...options }: Options<T> & { force?: boolean } = {}
): ProductRows<T> => {
  const groups = productGroups(items, productOf, options)
  if (groups.length === 0 || (groups.length === 1 && !force))
    return { rows: [...items], headerAt: () => undefined }
  const labels = new Map(groups.map(g => [g.key, g.label]))
  const starts = new Map<number, string>()
  let index = 0
  for (const group of groups) {
    starts.set(index, group.label)
    index += group.items.length
  }
  return {
    rows: groups.flatMap(g => g.items),
    groupOf: row => {
      const key = productOf(row)?.id ?? ''
      return { key, label: labels.get(key) ?? productGroupLabel(productOf(row), 0) }
    },
    headerAt: i => starts.get(i),
  }
}
