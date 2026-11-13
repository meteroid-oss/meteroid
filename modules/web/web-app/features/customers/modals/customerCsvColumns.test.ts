import { describe, expect, it } from 'vitest'

import ingestSource from '../../../../../meteroid/crates/meteroid-app/src/services/customer_ingest.rs?raw'

import {
  csvHeaders,
  customerCsvOptionalColumns,
  customerCsvRequiredColumns,
} from './customerCsvColumns'

/** Headers the backend reads: fields of the `*Csv` serde structs, `rename` winning, `flatten` expanded. */
function backendHeaders(source: string): string[] {
  const headers: string[] = []
  for (const [, body] of source.matchAll(/pub struct \w+Csv \{([\s\S]*?)\n\}/g)) {
    let attrsStart = 0
    for (const field of body.matchAll(/pub (\w+):[^\n]*/g)) {
      const attrs = body.slice(attrsStart, field.index)
      attrsStart = field.index + field[0].length
      if (attrs.includes('flatten')) continue
      headers.push(attrs.match(/rename = "([^"]+)"/)?.[1] ?? field[1])
    }
  }
  return headers.sort()
}

describe('customer CSV import columns', () => {
  it('lists exactly the columns the backend reads', () => {
    const backend = backendHeaders(ingestSource)
    expect(backend.length).toBeGreaterThan(20)

    const ui = csvHeaders([...customerCsvRequiredColumns, ...customerCsvOptionalColumns]).sort()
    expect(ui).toEqual(backend)
  })
})
