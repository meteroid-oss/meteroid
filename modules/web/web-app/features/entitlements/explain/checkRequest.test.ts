import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { ResourceSchema, ResourceTypeSchema } from '@/rpc/api/resources/v1/models_pb'

import { type Target, curlGet, curlPost, formatGet, restCheckBody, restCheckGet } from './checkRequest'

const tenant = create(ResourceTypeSchema, {
  id: 'rt_tenant',
  code: 'tenant',
  name: 'Tenant',
  pluralName: 'Tenants',
  keys: ['tenant_id'],
})
const stored: Target = {
  kind: 'stored',
  type: tenant,
  resource: create(ResourceSchema, { id: 'res_1', keyValue: 'halyard-prod', resourceTypeId: tenant.id }),
}
const adhoc: Target = { kind: 'adhoc', type: tenant, keyValue: 'new one', attributes: { environment: 'staging', region: '' } }

describe('restCheckBody', () => {
  it('names the resource by type and key, with the amounts given', () => {
    expect(restCheckBody('alerts', stored, { requested: '1', currentCount: '18' })).toEqual({
      feature: 'alerts',
      resource: { type: 'tenant', key: 'halyard-prod' },
      requested: '1',
      current_count: '18',
      explain: true,
    })
  })

  it("asserts an ad-hoc resource's attributes, leaving blank ones out", () => {
    expect(restCheckBody('alerts', adhoc)).toEqual({
      feature: 'alerts',
      resource: { type: 'tenant', key: 'new one' },
      attributes: { 'tenant.environment': 'staging' },
      explain: true,
    })
  })

  it('is the feature alone for the whole customer', () => {
    expect(restCheckBody('alerts', { kind: 'customer' })).toEqual({ feature: 'alerts', explain: true })
  })
})

describe('restCheckGet', () => {
  it('reads the same check as query parameters', () => {
    const get = restCheckGet('cus_1', 'alerts', adhoc, { requested: '2' })
    expect(get.path).toBe('/api/v2/customers/cus_1/entitlements/alerts/check')
    expect(get.params).toEqual([
      'resource[type]=tenant',
      'resource[key]=new%20one',
      'attributes[tenant.environment]=staging',
      'requested=2',
      'explain=true',
    ])
    expect(formatGet(get)).toBe(
      [
        '/api/v2/customers/cus_1/entitlements/alerts/check',
        '    ?resource[type]=tenant',
        '    &resource[key]=new%20one',
        '    &attributes[tenant.environment]=staging',
        '    &requested=2',
        '    &explain=true',
      ].join('\n')
    )
  })
})

describe('curl', () => {
  it("quotes the body and the URL for a shell, a single quote included", () => {
    const body = restCheckBody("o'clock", { kind: 'customer' })
    expect(curlPost('https://api.test', 'cus_1', body)).toBe(
      [
        'curl -X POST https://api.test/api/v2/customers/cus_1/entitlements/check \\',
        '  -H "Authorization: Bearer $METEROID_API_KEY" -H "Content-Type: application/json" \\',
        `  -d '{"feature":"o'\\''clock","explain":true}'`,
      ].join('\n')
    )
    expect(curlGet('https://api.test', restCheckGet('cus_1', 'alerts', { kind: 'customer' }))).toBe(
      [
        "curl 'https://api.test/api/v2/customers/cus_1/entitlements/alerts/check?explain=true' \\",
        '  -H "Authorization: Bearer $METEROID_API_KEY"',
      ].join('\n')
    )
  })
})
