import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { ProductRefSchema } from '@/rpc/api/entitlements/v1/models_pb'
import { UsageLimitStatusSchema } from '@/rpc/portal/customer/v1/models_pb'

import { limitSections } from './limits'

const limit = (id: string, product?: { id: string; name: string }) =>
  create(UsageLimitStatusSchema, {
    id,
    name: id,
    limit: '100',
    product: product && create(ProductRefSchema, product),
  })

const api = { id: 'p-api', name: 'API' }
const storage = { id: 'p-storage', name: 'Storage' }

describe('limitSections', () => {
  it('heads the limits of each product, tenant-wide ones last', () => {
    const sections = limitSections([limit('a'), limit('b', storage), limit('c', api), limit('d', storage)])
    expect(sections.map(s => s.heading)).toEqual(['API · 1', 'Storage · 2', 'All products · 1'])
    expect(sections.map(s => s.limits.map(l => l.id))).toEqual([['c'], ['b', 'd'], ['a']])
  })

  it('keeps one unheaded group when every limit has the same product', () => {
    const sections = limitSections([limit('b', api), limit('a', api)])
    expect(sections).toHaveLength(1)
    expect(sections[0].heading).toBeUndefined()
    expect(sections[0].limits.map(l => l.id)).toEqual(['b', 'a'])
  })

  it('keeps one unheaded group when no limit has a product', () => {
    const sections = limitSections([limit('a'), limit('b')])
    expect(sections).toEqual([{ key: '', limits: sections[0].limits }])
    expect(sections[0].heading).toBeUndefined()
  })
})
