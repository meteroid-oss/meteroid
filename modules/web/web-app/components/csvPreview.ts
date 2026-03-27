import { parse } from 'papaparse'
import { z } from 'zod'

import type { ReactNode } from 'react'

export interface ColumnDefinition {
  name: string
  tooltipMessage?: ReactNode
}

export interface CellError {
  rowIndex: number
  columnIndex: number
  message: string
}

export interface CSVPreview {
  headers: string[]
  rows: string[][]
  totalRowEstimate: number
  headerValidation: HeaderValidation
  rowErrors: CellError[]
}

export interface HeaderValidation {
  missingRequired: string[]
  matchedRequired: string[]
  matchedOptional: string[]
  unknown: string[]
}

const PREVIEW_ROWS = 10
// Read first 64KB — enough for headers + preview rows in any reasonable CSV
const PREVIEW_BYTE_LIMIT = 64 * 1024

export function matchesColumnDef(header: string, col: ColumnDefinition): boolean {
  const h = header.toLowerCase()
  const name = col.name.toLowerCase()
  if (name.endsWith('.*')) {
    return h.startsWith(name.slice(0, -1))
  }
  if (name === '(additional)') return false
  return h === name
}

export function matchesAnyColumnDef(header: string, cols: ColumnDefinition[]): boolean {
  return cols.some(col => matchesColumnDef(header, col))
}

function validateHeaders(
  headers: string[],
  requiredColumns: ColumnDefinition[],
  optionalColumns: ColumnDefinition[]
): HeaderValidation {
  const matchedRequired: string[] = []
  const missingRequired: string[] = []
  const matchedOptional: string[] = []
  const unknown: string[] = []

  for (const col of requiredColumns) {
    if (headers.some(h => matchesColumnDef(h, col))) {
      matchedRequired.push(col.name)
    } else {
      missingRequired.push(col.name)
    }
  }

  for (const header of headers) {
    const isRequired = requiredColumns.some(col => matchesColumnDef(header, col))
    const isOptional = optionalColumns.some(col => matchesColumnDef(header, col))
    if (isOptional && !isRequired) {
      matchedOptional.push(header)
    } else if (!isRequired && !isOptional) {
      const hasAdditional = optionalColumns.some(c => c.name === '(additional)')
      if (!hasAdditional) {
        unknown.push(header)
      }
    }
  }

  return { missingRequired, matchedRequired, matchedOptional, unknown }
}

function validateRows(
  headers: string[],
  rows: string[][],
  rowSchema: z.ZodObject<z.ZodRawShape>
): CellError[] {
  const errors: CellError[] = []
  const headerToIndex = new Map(headers.map((h, i) => [h, i]))

  for (let rowIdx = 0; rowIdx < rows.length; rowIdx++) {
    const row = rows[rowIdx]
    const obj: Record<string, string> = {}
    for (let i = 0; i < headers.length; i++) {
      obj[headers[i]] = row[i] ?? ''
    }

    const result = rowSchema.safeParse(obj)
    if (!result.success) {
      for (const issue of result.error.issues) {
        const fieldName = issue.path[0] as string
        const colIdx = headerToIndex.get(fieldName)
        if (colIdx !== undefined) {
          errors.push({ rowIndex: rowIdx, columnIndex: colIdx, message: issue.message })
        }
      }
    }
  }

  return errors
}

function estimateRowCount(fileSize: number, sampleBytes: number, sampleDataLines: number): number {
  if (sampleDataLines === 0) return 0
  const avgBytesPerLine = sampleBytes / (sampleDataLines + 1) // +1 for header
  return Math.max(sampleDataLines, Math.round(fileSize / avgBytesPerLine) - 1)
}

export async function parseCSVPreview(
  file: File,
  delimiter: string,
  requiredColumns: ColumnDefinition[],
  optionalColumns: ColumnDefinition[],
  rowSchema?: z.ZodObject<z.ZodRawShape>
): Promise<CSVPreview> {
  const isSliced = file.size > PREVIEW_BYTE_LIMIT
  const slice = isSliced ? file.slice(0, PREVIEW_BYTE_LIMIT) : file
  const text = await slice.text()

  const parsed = parse<string[]>(text, {
    delimiter,
    header: false,
    skipEmptyLines: true,
  })

  let allRows = parsed.data
  // Discard last row if we sliced — it may be truncated at the byte boundary
  if (isSliced && allRows.length > 1) {
    allRows = allRows.slice(0, -1)
  }

  if (allRows.length === 0) {
    return {
      headers: [],
      rows: [],
      totalRowEstimate: 0,
      headerValidation: {
        missingRequired: requiredColumns.map(c => c.name),
        matchedRequired: [],
        matchedOptional: [],
        unknown: [],
      },
      rowErrors: [],
    }
  }

  // Filter empty headers (e.g., trailing delimiter) and realign row data
  const rawHeaders = allRows[0].map(h => h.trim())
  const validIndices = rawHeaders.reduce<number[]>((acc, h, i) => {
    if (h.length > 0) acc.push(i)
    return acc
  }, [])
  const headers = validIndices.map(i => rawHeaders[i])
  const dataRows = allRows.slice(1)
  const previewRows = dataRows
    .slice(0, PREVIEW_ROWS)
    .map(row => validIndices.map(i => row[i] ?? ''))

  const headerValidation = validateHeaders(headers, requiredColumns, optionalColumns)
  const totalRowEstimate = isSliced
    ? estimateRowCount(file.size, text.length, dataRows.length)
    : dataRows.length

  const rowErrors =
    rowSchema && headerValidation.missingRequired.length === 0
      ? validateRows(headers, previewRows, rowSchema)
      : []

  return { headers, rows: previewRows, totalRowEstimate, headerValidation, rowErrors }
}
