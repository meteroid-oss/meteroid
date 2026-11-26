import type { KindKey } from '@/features/entitlements/features/featureKind'
import type { ResourceType } from '@/rpc/api/resources/v1/models_pb'

type SnippetInput = {
  code: string
  kind: KindKey
  countedBy: 'EVENTS' | 'APP'
  type?: ResourceType
  /** The REST API's external URL. */
  baseUrl: string
}

const IDENTIFIER = /^[A-Za-z_$][\w$]*$/

const variableOf = (type: ResourceType) => {
  const name = type.code.replace(/[^\w$]/g, '_')
  return IDENTIFIER.test(name) ? name : `${type.code ? '_' : 'resource'}${name}`
}

// POST /api/v2/customers/{id_or_alias}/entitlements/check, with fetch: no SDK method exists for it.
export function featureSnippet({ code, kind, countedBy, type, baseUrl }: SnippetInput): string {
  const fields = [`feature: ${JSON.stringify(code || 'feature_code')}`]
  if (type) fields.push(`resource: { type: ${JSON.stringify(type.code)}, key: ${variableOf(type)}.id }`)
  if (kind === 'LIMIT' && countedBy === 'APP') fields.push('current_count: String(currentCount)')
  if (kind === 'LIMIT') fields.push(`requested: '1'`)

  const result =
    kind === 'VALUE' ? '{ value }' : kind === 'LIMIT' ? '{ allowed, remaining }' : '{ allowed }'
  return [
    `const res = await fetch(\`${baseUrl}/api/v2/customers/\${customer.id}/entitlements/check\`, {`,
    `  method: 'POST',`,
    `  headers: {`,
    `    Authorization: \`Bearer \${process.env.METEROID_API_KEY}\`,`,
    `    'Content-Type': 'application/json',`,
    `  },`,
    `  body: JSON.stringify({`,
    ...fields.map(f => `    ${f},`),
    `  }),`,
    `})`,
    `const ${result} = await res.json()`,
  ].join('\n')
}
