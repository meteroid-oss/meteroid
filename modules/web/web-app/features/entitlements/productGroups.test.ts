import { describe, expect, it } from 'vitest'

import { productGroups, productRows, productSections } from './productGroups'

type Item = { id: string; product?: { id: string; name: string } }

const api = { id: 'p-api', name: 'API' }
const seats = { id: 'p-seats', name: 'seats' }
const analytics = { id: 'p-analytics', name: 'Analytics' }

const item = (id: string, product?: Item['product']): Item => ({ id, product })
const productOf = (i: Item) => i.product
const ids = (items: Item[]) => items.map(i => i.id)

describe('productGroups', () => {
  it('orders products by name, case-insensitively, and tenant-wide features last', () => {
    const groups = productGroups(
      [item('a', seats), item('b'), item('c', api), item('d', analytics), item('e', seats)],
      productOf
    )
    expect(groups.map(g => g.label)).toEqual([
      'Analytics · 1',
      'API · 1',
      'seats · 2',
      'All products · 1',
    ])
    expect(groups.map(g => g.key)).toEqual(['p-analytics', 'p-api', 'p-seats', ''])
    expect(groups.map(g => g.name)).toEqual(['Analytics', 'API', 'seats', 'All products'])
  })

  it("keeps the view's order inside a group", () => {
    const groups = productGroups([item('z', api), item('a'), item('m', api), item('b', api)], productOf)
    expect(ids(groups[0].items)).toEqual(['z', 'm', 'b'])
  })

  it('tells apart two products of the same name', () => {
    const twin = { id: 'p-api-2', name: 'API' }
    const groups = productGroups([item('a', twin), item('b', api)], productOf)
    expect(groups.map(g => g.key)).toEqual(['p-api', 'p-api-2'])
  })

  it('counts the rows of one feature once', () => {
    const rows = [
      { feature: 'f1', product: api },
      { feature: 'f1', product: api },
      { feature: 'f2', product: api },
    ]
    const [group] = productGroups(rows, r => r.product, { countBy: r => r.feature })
    expect(group.label).toBe('API · 2')
    expect(group.items).toHaveLength(3)
  })

  it('is empty without items', () => {
    expect(productGroups([], productOf)).toEqual([])
  })
})

describe('productSections', () => {
  it('renders flat when every item shares a product', () => {
    expect(productSections([item('a', api), item('b', api)], productOf)).toBeUndefined()
  })

  it('renders flat when every item is tenant-wide', () => {
    expect(productSections([item('a'), item('b')], productOf)).toBeUndefined()
  })

  it('heads every group once two differ', () => {
    expect(productSections([item('a'), item('b', api)], productOf)?.map(g => g.label)).toEqual([
      'API · 1',
      'All products · 1',
    ])
  })
})

describe('productRows', () => {
  it('reorders the rows by group and labels each', () => {
    const { rows, groupOf } = productRows([item('a'), item('b', api), item('c'), item('d', api)], productOf)
    expect(ids(rows)).toEqual(['b', 'd', 'a', 'c'])
    expect(rows.map(r => groupOf!(r).label)).toEqual(['API · 2', 'API · 2', 'All products · 2', 'All products · 2'])
  })

  it('heads the first row of each group', () => {
    const { headerAt } = productRows([item('a'), item('b', api), item('c'), item('d', api)], productOf)
    expect([0, 1, 2, 3].map(headerAt)).toEqual(['API · 2', undefined, 'All products · 2', undefined])
  })

  it('leaves a single group flat, in its order', () => {
    const { rows, groupOf, headerAt } = productRows([item('b', api), item('a', api)], productOf)
    expect(ids(rows)).toEqual(['b', 'a'])
    expect(groupOf).toBeUndefined()
    expect(headerAt(0)).toBeUndefined()
  })

  it('heads a single group when forced', () => {
    const { groupOf } = productRows([item('a', api)], productOf, { force: true })
    expect(groupOf!(item('a', api))).toEqual({ key: 'p-api', label: 'API · 1' })
  })
})
