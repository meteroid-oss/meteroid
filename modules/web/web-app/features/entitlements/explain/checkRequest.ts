import { resourceLabel } from '@/features/resources/hooks'

import type { Resource, ResourceType } from '@/rpc/api/resources/v1/models_pb'

/** What a check names: the customer alone, a stored resource, or an ad-hoc one with typed attributes. */
export type Target =
  | { kind: 'customer' }
  | { kind: 'stored'; type: ResourceType; resource: Resource }
  | { kind: 'adhoc'; type: ResourceType; keyValue: string; attributes: Record<string, string> }

/** Decimal strings; unset = the server's own (1 requested, the stored count). */
export type CheckAmounts = { requested?: string; currentCount?: string }

export const targetRequest = (t: Target) =>
  t.kind === 'customer'
    ? { resources: {}, attributes: {} }
    : {
        resources: { [t.type.keys[0]]: t.kind === 'stored' ? t.resource.keyValue : t.keyValue },
        attributes:
          t.kind === 'adhoc'
            ? Object.fromEntries(
                Object.entries(t.attributes)
                  .filter(([, v]) => v)
                  .map(([k, v]) => [`${t.type.code}.${k}`, v])
              )
            : {},
      }

export const targetLabel = (t: Target, customerName: string) =>
  t.kind === 'customer'
    ? customerName
    : `${t.type.name.toLowerCase()} ${t.kind === 'stored' ? resourceLabel(t.resource) : t.keyValue || '…'}`

/** An ad-hoc target without its key can't be checked yet. */
export const isTargetReady = (t: Target) => !(t.kind === 'adhoc' && !t.keyValue)

const targetResource = (t: Target) =>
  t.kind === 'customer' ? undefined : { type: t.type.code, key: t.kind === 'stored' ? t.resource.keyValue : t.keyValue }

/** The same check as `POST /api/v2/customers/{id_or_alias}/entitlements/check` takes it. */
export const restCheckBody = (feature: string, t: Target, amounts: CheckAmounts = {}) => {
  const resource = targetResource(t)
  const { attributes } = targetRequest(t)
  return {
    feature,
    ...(resource ? { resource } : {}),
    ...(Object.keys(attributes).length ? { attributes } : {}),
    ...(amounts.requested !== undefined ? { requested: amounts.requested } : {}),
    ...(amounts.currentCount !== undefined ? { current_count: amounts.currentCount } : {}),
    explain: true,
  }
}

export const restCheckPostPath = (customerId: string) =>
  `/api/v2/customers/${encodeURIComponent(customerId)}/entitlements/check`

/** `GET /api/v2/customers/{id_or_alias}/entitlements/{feature}/check`'s path and query parameters. */
export const restCheckGet = (customerId: string, feature: string, t: Target, amounts: CheckAmounts = {}) => {
  const resource = targetResource(t)
  const { attributes } = targetRequest(t)
  const params: [string, string][] = [
    ...(resource ? ([['resource[type]', resource.type], ['resource[key]', resource.key]] as [string, string][]) : []),
    ...Object.entries(attributes).map(([k, v]): [string, string] => [`attributes[${k}]`, v]),
    ...(amounts.requested !== undefined ? ([['requested', amounts.requested]] as [string, string][]) : []),
    ...(amounts.currentCount !== undefined ? ([['current_count', amounts.currentCount]] as [string, string][]) : []),
    ['explain', 'true'],
  ]
  return {
    path: `/api/v2/customers/${encodeURIComponent(customerId)}/entitlements/${encodeURIComponent(feature)}/check`,
    params: params.map(([k, v]) => `${k}=${encodeURIComponent(v)}`),
  }
}

/** The path, then one parameter a line, as the request panel shows it. */
export const formatGet = ({ path, params }: ReturnType<typeof restCheckGet>) =>
  [path, ...params.map((p, i) => `    ${i === 0 ? '?' : '&'}${p}`)].join('\n')

const shellQuote = (s: string) => `'${s.replace(/'/g, `'\\''`)}'`

const AUTH = `  -H "Authorization: Bearer $METEROID_API_KEY"`

export const curlPost = (baseUrl: string, customerId: string, body: object) =>
  [
    `curl -X POST ${baseUrl}${restCheckPostPath(customerId)} \\`,
    `${AUTH} -H "Content-Type: application/json" \\`,
    `  -d ${shellQuote(JSON.stringify(body))}`,
  ].join('\n')

export const curlGet = (baseUrl: string, { path, params }: ReturnType<typeof restCheckGet>) =>
  [`curl ${shellQuote(`${baseUrl}${path}?${params.join('&')}`)} \\`, AUTH].join('\n')
