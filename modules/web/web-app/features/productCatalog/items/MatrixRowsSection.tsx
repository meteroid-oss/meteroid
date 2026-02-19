import { createConnectQueryKey, disableQuery, useMutation } from '@connectrpc/connect-query'
import {
  Badge,
  Button,
  Checkbox,
  Input,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { toast } from 'sonner'

import { useQuery } from '@/lib/connectrpc'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import type { BillableMetric } from '@/rpc/api/billablemetrics/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import { UsagePricing_MatrixPricing_MatrixDimension } from '@/rpc/api/prices/v1/models_pb'
import {
  listPricesByProduct,
  previewMatrixUpdate,
  updateMatrixPrices,
} from '@/rpc/api/prices/v1/prices-PricesService_connectquery'
import type { PreviewMatrixUpdateResponse } from '@/rpc/api/prices/v1/prices_pb'
import { MatrixDimensionKey, MatrixRowAdd } from '@/rpc/api/prices/v1/prices_pb'

interface MatrixRowsSectionProps {
  productId: string
  metricId: string
  currencies: string[]
}

interface DimensionCombo {
  d1Key: string
  d1Value: string
  d2Key?: string
  d2Value?: string
}

type RowStatus = 'active' | 'missing' | 'orphaned'

interface DisplayRow {
  combo: DimensionCombo
  status: RowStatus
}

function comboKey(c: DimensionCombo): string {
  return `${c.d1Key}:${c.d1Value}|${c.d2Key ?? ''}:${c.d2Value ?? ''}`
}

function extractMatrixRows(price: Price): DimensionCombo[] {
  if (price.pricing.case !== 'usagePricing') return []
  const model = price.pricing.value.model
  if (model.case !== 'matrix') return []
  return model.value.rows.map(r => ({
    d1Key: r.dimension1?.key ?? '',
    d1Value: r.dimension1?.value ?? '',
    d2Key: r.dimension2?.key,
    d2Value: r.dimension2?.value,
  }))
}

function useMatrixDimensions(metric: BillableMetric | undefined): {
  headers: string[]
  validCombos: DimensionCombo[]
} {
  return useMemo(() => {
    const seg = metric?.segmentationMatrix
    if (!seg?.matrix) return { headers: [], validCombos: [] }

    const matrix = seg.matrix
    if (matrix.case === 'single') {
      const dim = matrix.value?.dimension
      const key = dim?.key ?? ''
      return {
        headers: [key],
        validCombos: (dim?.values ?? []).map(v => ({ d1Key: key, d1Value: v })),
      }
    }
    if (matrix.case === 'double') {
      const d1 = matrix.value?.dimension1
      const d2 = matrix.value?.dimension2
      const k1 = d1?.key ?? ''
      const k2 = d2?.key ?? ''
      return {
        headers: [k1, k2],
        validCombos: (d1?.values ?? []).flatMap(v1 =>
          (d2?.values ?? []).map(v2 => ({ d1Key: k1, d1Value: v1, d2Key: k2, d2Value: v2 }))
        ),
      }
    }
    if (matrix.case === 'linked') {
      const k1 = matrix.value.dimensionKey
      const k2 = matrix.value.linkedDimensionKey
      return {
        headers: [k1, k2],
        validCombos: Object.entries(matrix.value.values).flatMap(([k, v]) =>
          v.values.map(linkedV => ({ d1Key: k1, d1Value: k, d2Key: k2, d2Value: linkedV }))
        ),
      }
    }
    return { headers: [], validCombos: [] }
  }, [metric])
}

function buildDisplayRows(
  priceCombos: DimensionCombo[],
  validCombos: DimensionCombo[]
): DisplayRow[] {
  const priceSet = new Set(priceCombos.map(comboKey))
  const validSet = new Set(validCombos.map(comboKey))
  const rows: DisplayRow[] = []

  for (const combo of priceCombos) {
    rows.push({
      combo,
      status: validSet.has(comboKey(combo)) ? 'active' : 'orphaned',
    })
  }

  for (const combo of validCombos) {
    if (!priceSet.has(comboKey(combo))) {
      rows.push({ combo, status: 'missing' })
    }
  }

  return rows
}

function makeDimension(key: string, value: string) {
  return new UsagePricing_MatrixPricing_MatrixDimension({ key, value })
}

function makeRowAdd(combo: DimensionCombo, perUnitPrices: Record<string, string>) {
  return new MatrixRowAdd({
    dimension1: makeDimension(combo.d1Key, combo.d1Value),
    dimension2:
      combo.d2Key && combo.d2Value ? makeDimension(combo.d2Key, combo.d2Value) : undefined,
    perUnitPrices,
  })
}

function makeDimensionKey(combo: DimensionCombo) {
  return new MatrixDimensionKey({
    dimension1: makeDimension(combo.d1Key, combo.d1Value),
    dimension2:
      combo.d2Key && combo.d2Value ? makeDimension(combo.d2Key, combo.d2Value) : undefined,
  })
}

function priceKey(combo: DimensionCombo, currency: string): string {
  return `${comboKey(combo)}||${currency}`
}

type Step = 'idle' | 'select' | 'preview'

export const MatrixRowsSection = ({ productId, metricId, currencies }: MatrixRowsSectionProps) => {
  const queryClient = useQueryClient()

  const metricQuery = useQuery(getBillableMetric, metricId ? { id: metricId } : disableQuery, {
    refetchOnWindowFocus: 'always',
    refetchOnMount: 'always',
  })
  const pricesQuery = useQuery(
    listPricesByProduct,
    { productId },
    { refetchOnWindowFocus: 'always', refetchOnMount: 'always' }
  )

  const { headers, validCombos } = useMatrixDimensions(metricQuery.data?.billableMetric)
  const prices = pricesQuery.data?.prices ?? []

  const priceCombos = useMemo(() => {
    const matrixPrice = prices.find(
      p => p.pricing.case === 'usagePricing' && p.pricing.value.model.case === 'matrix'
    )
    return matrixPrice ? extractMatrixRows(matrixPrice) : []
  }, [prices])

  const displayRows = useMemo(
    () => buildDisplayRows(priceCombos, validCombos),
    [priceCombos, validCombos]
  )

  const missingRows = displayRows.filter(r => r.status === 'missing')
  const orphanedRows = displayRows.filter(r => r.status === 'orphaned')
  const activeRows = displayRows.filter(r => r.status === 'active')

  // Expand/collapse
  const [expanded, setExpanded] = useState(false)

  // Step machine: idle → select → preview
  const [step, setStep] = useState<Step>('idle')

  // Selection state (set of comboKeys)
  const [selected, setSelected] = useState<Set<string>>(new Set())

  // Per-row, per-currency price inputs
  const [rowPrices, setRowPrices] = useState<Record<string, string>>({})
  const [bulkPrices, setBulkPrices] = useState<Record<string, string>>({})

  // Preview result
  const [previewResult, setPreviewResult] = useState<PreviewMatrixUpdateResponse | null>(null)

  const toggleSelected = useCallback((combo: DimensionCombo) => {
    const key = comboKey(combo)
    setSelected(prev => {
      const next = new Set(prev)
      if (next.has(key)) {
        next.delete(key)
      } else {
        next.add(key)
      }
      return next
    })
  }, [])

  const selectAll = useCallback(() => {
    setSelected(new Set(missingRows.map(r => comboKey(r.combo))))
  }, [missingRows])

  const selectNone = useCallback(() => {
    setSelected(new Set())
  }, [])

  const getRowPrice = useCallback(
    (combo: DimensionCombo, currency: string) => rowPrices[priceKey(combo, currency)] ?? '',
    [rowPrices]
  )

  const setRowPrice = useCallback((combo: DimensionCombo, currency: string, price: string) => {
    setRowPrices(prev => ({ ...prev, [priceKey(combo, currency)]: price }))
  }, [])

  const getBulkPrice = useCallback((currency: string) => bulkPrices[currency] ?? '', [bulkPrices])

  const setBulkPrice = useCallback((currency: string, price: string) => {
    setBulkPrices(prev => ({ ...prev, [currency]: price }))
  }, [])

  const buildPricesMap = useCallback(
    (combo: DimensionCombo): Record<string, string> => {
      const map: Record<string, string> = {}
      for (const currency of currencies) {
        map[currency] = rowPrices[priceKey(combo, currency)] || bulkPrices[currency] || '0'
      }
      return map
    },
    [currencies, rowPrices, bulkPrices]
  )

  const selectedCombos = useMemo(
    () => missingRows.filter(r => selected.has(comboKey(r.combo))).map(r => r.combo),
    [missingRows, selected]
  )

  const invalidate = async () => {
    await queryClient.invalidateQueries({
      queryKey: createConnectQueryKey(listPricesByProduct, { productId }),
    })
  }

  const reset = () => {
    setStep('idle')
    setSelected(new Set())
    setRowPrices({})
    setBulkPrices({})
    setPreviewResult(null)
  }

  const previewMutation = useMutation(previewMatrixUpdate, {
    onSuccess: data => {
      setPreviewResult(data)
      setStep('preview')
    },
    onError: () => {
      toast.error('Failed to load preview')
    },
  })

  const applyMutation = useMutation(updateMatrixPrices, {
    onSuccess: async () => {
      await invalidate()
      toast.success('Matrix rows updated')
      reset()
    },
    onError: () => toast.error('Failed to apply matrix update'),
  })

  // Start the "add missing" flow
  const startAddMissing = () => {
    setStep('select')
    setSelected(new Set(missingRows.map(r => comboKey(r.combo))))
    setExpanded(true)
  }

  // Request preview for selected rows
  const requestPreview = () => {
    if (selectedCombos.length === 0) return
    previewMutation.mutate({
      productId,
      addRows: selectedCombos.map(makeDimensionKey),
      removeRows: [],
    })
  }

  // Apply after preview
  const applyAdd = () => {
    applyMutation.mutate({
      productId,
      addRows: selectedCombos.map(c => makeRowAdd(c, buildPricesMap(c))),
      removeRows: [],
    })
  }

  // Remove orphaned (direct: preview → apply)
  const [removePreview, setRemovePreview] = useState<PreviewMatrixUpdateResponse | null>(null)

  const removePreviewMutation = useMutation(previewMatrixUpdate, {
    onSuccess: data => setRemovePreview(data),
    onError: () => toast.error('Failed to load preview'),
  })

  const removeApplyMutation = useMutation(updateMatrixPrices, {
    onSuccess: async () => {
      await invalidate()
      toast.success('Orphaned rows removed')
      setRemovePreview(null)
    },
    onError: () => toast.error('Failed to remove orphaned rows'),
  })

  const requestRemoveOrphaned = () => {
    removePreviewMutation.mutate({
      productId,
      addRows: [],
      removeRows: orphanedRows.map(r => makeDimensionKey(r.combo)),
    })
  }

  const applyRemoveOrphaned = () => {
    removeApplyMutation.mutate({
      productId,
      addRows: [],
      removeRows: orphanedRows.map(r => makeDimensionKey(r.combo)),
    })
  }

  const isLoading = metricQuery.isLoading || pricesQuery.isLoading
  const isBusy =
    previewMutation.isPending ||
    applyMutation.isPending ||
    removePreviewMutation.isPending ||
    removeApplyMutation.isPending
  const hasTwoDimensions = headers.length >= 2

  if (isLoading) {
    return (
      <section className="flex flex-col gap-3">
        <h3 className="text-sm font-medium text-muted-foreground">Matrix Dimensions</h3>
        <Skeleton className="h-20 w-full" />
      </section>
    )
  }

  if (displayRows.length === 0 && validCombos.length === 0) {
    return (
      <section className="flex flex-col gap-3">
        <h3 className="text-sm font-medium text-muted-foreground">Matrix Dimensions</h3>
        <p className="text-sm text-muted-foreground">
          No dimension combinations defined on the linked metric.
        </p>
      </section>
    )
  }

  return (
    <section className="flex flex-col gap-3">
      {/* Header with summary + expand toggle */}
      <button
        type="button"
        className="flex items-center gap-2 text-sm font-medium text-muted-foreground hover:text-foreground transition-colors"
        onClick={() => setExpanded(e => !e)}
      >
        {expanded ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
        <span>Matrix Dimensions</span>
        <span className="text-xs font-normal">
          {activeRows.length} active
          {missingRows.length > 0 && (
            <>
              , <span className="text-warning">{missingRows.length} missing</span>
            </>
          )}
          {orphanedRows.length > 0 && (
            <>
              , <span className="text-destructive">{orphanedRows.length} orphaned</span>
            </>
          )}
        </span>
      </button>

      {/* Action buttons (visible even when collapsed) */}
      {expanded && step === 'idle' && (missingRows.length > 0 || orphanedRows.length > 0) && (
        <div className="flex items-center gap-2">
          {missingRows.length > 0 && !removePreview && (
            <Button variant="outline" size="sm" disabled={isBusy} onClick={startAddMissing}>
              Add missing ({missingRows.length})
            </Button>
          )}
          {orphanedRows.length > 0 && !removePreview && (
            <Button variant="outline" size="sm" disabled={isBusy} onClick={requestRemoveOrphaned}>
              Remove orphaned ({orphanedRows.length})
            </Button>
          )}
        </div>
      )}

      {/* Remove orphaned preview banner */}
      {removePreview && (
        <PreviewBanner
          preview={removePreview}
          label={`Remove ${orphanedRows.length} orphaned row(s)`}
          isBusy={removeApplyMutation.isPending}
          onApply={applyRemoveOrphaned}
          onCancel={() => setRemovePreview(null)}
        />
      )}

      {/* Selection step */}
      {step === 'select' && expanded && (
        <div className="flex flex-col gap-3">
          <div className="text-xs text-muted-foreground   border-l border-muted-foreground/30 pl-2">
            Update immediately all prices with these new rows. No plan migration necessary.
            <br />
            Prices where these rows are already defined will be unaffected.
          </div>

          <div className="flex items-center gap-3">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <span>
                {selected.size} / {missingRows.length} selected
              </span>
              <Button variant="link" size="sm" className="h-auto p-0 text-xs" onClick={selectAll}>
                All
              </Button>
              <Button variant="link" size="sm" className="h-auto p-0 text-xs" onClick={selectNone}>
                None
              </Button>
            </div>
          </div>

          {/* Default prices for bulk */}
          {currencies.length > 0 && (
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <span>Default price:</span>
              {currencies.map(currency => (
                <div key={currency} className="flex items-center gap-1">
                  <span className="font-mono">{currency}</span>
                  <Input
                    type="number"
                    min="0"
                    step="0.01"
                    value={getBulkPrice(currency)}
                    onChange={e => setBulkPrice(currency, e.target.value)}
                    className="w-24 h-7 text-xs"
                    placeholder="0.00"
                  />
                </div>
              ))}
            </div>
          )}

          {/* Missing rows table with checkboxes */}
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-8" />
                <TableHead>{headers[0] || 'Dimension 1'}</TableHead>
                {hasTwoDimensions && <TableHead>{headers[1]}</TableHead>}
                {currencies.map(currency => (
                  <TableHead key={currency}>
                    <span className="font-mono">{currency}</span>
                  </TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {missingRows.map(({ combo }) => {
                const key = comboKey(combo)
                const isSelected = selected.has(key)
                return (
                  <TableRow key={key} className={isSelected ? '' : 'opacity-50'}>
                    <TableCell>
                      <Checkbox
                        checked={isSelected}
                        onCheckedChange={() => toggleSelected(combo)}
                      />
                    </TableCell>
                    <TableCell className="font-mono text-xs">{combo.d1Value}</TableCell>
                    {hasTwoDimensions && (
                      <TableCell className="font-mono text-xs">{combo.d2Value}</TableCell>
                    )}
                    {currencies.map(currency => (
                      <TableCell key={currency}>
                        {isSelected && (
                          <Input
                            type="number"
                            min="0"
                            step="0.01"
                            value={getRowPrice(combo, currency)}
                            onChange={e => setRowPrice(combo, currency, e.target.value)}
                            className="w-24 h-7 text-xs"
                            placeholder={getBulkPrice(currency) || '0.00'}
                          />
                        )}
                      </TableCell>
                    ))}
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>

          <div className="flex items-center gap-2">
            <Button
              variant="default"
              size="sm"
              disabled={selectedCombos.length === 0 || isBusy}
              onClick={requestPreview}
            >
              {previewMutation.isPending ? 'Loading preview...' : 'Preview'}
            </Button>
            <Button variant="outline" size="sm" disabled={isBusy} onClick={reset}>
              Cancel
            </Button>
          </div>
        </div>
      )}

      {/* Preview step */}
      {step === 'preview' && previewResult && (
        <PreviewBanner
          preview={previewResult}
          label={`Add ${selectedCombos.length} dimension row(s)`}
          isBusy={applyMutation.isPending}
          onApply={applyAdd}
          onCancel={reset}
        />
      )}

      {/* Expanded dimension overview (when idle) */}
      {expanded && step === 'idle' && displayRows.length > 0 && (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{headers[0] || 'Dimension 1'}</TableHead>
              {hasTwoDimensions && <TableHead>{headers[1]}</TableHead>}
              <TableHead>Status</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {displayRows.map(({ combo, status }) => (
              <TableRow key={comboKey(combo)}>
                <TableCell className="font-mono text-xs">{combo.d1Value}</TableCell>
                {hasTwoDimensions && (
                  <TableCell className="font-mono text-xs">{combo.d2Value}</TableCell>
                )}
                <TableCell>
                  <StatusBadge status={status} />
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </section>
  )
}

function PreviewBanner({
  preview,
  label,
  isBusy,
  onApply,
  onCancel,
}: {
  preview: PreviewMatrixUpdateResponse
  label: string
  isBusy: boolean
  onApply: () => void
  onCancel: () => void
}) {
  return (
    <div className="rounded-md border border-border bg-muted/50 p-3 flex flex-col gap-2">
      <p className="text-sm font-medium">{label}</p>
      <div className="text-xs text-muted-foreground flex flex-col gap-1">
        {(preview.rowsToAdd > 0 || preview.rowsToRemove > 0) && (
          <span>
            {preview.rowsToAdd > 0 && `${preview.rowsToAdd} row(s) to add`}
            {preview.rowsToAdd > 0 && preview.rowsToRemove > 0 && ', '}
            {preview.rowsToRemove > 0 && `${preview.rowsToRemove} row(s) to remove`}
          </span>
        )}
        <span>
          Across {preview.affectedPrices} price(s), {preview.affectedSubscriptions} subscription(s)
          affected
        </span>
        {preview.affectedPlans.length > 0 && (
          <div className="mt-1">
            <span className="font-medium text-foreground">Impacted plans:</span>
            <ul className="ml-4 mt-0.5">
              {preview.affectedPlans.map(plan => (
                <li key={plan.planName}>
                  {plan.planName}
                  {plan.versions.length > 0 && (
                    <span className="text-muted-foreground">
                      {' '}
                      (<VersionList versions={plan.versions} />)
                    </span>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
      <div className="flex items-center gap-2 pt-1">
        <Button variant="default" size="sm" disabled={isBusy} onClick={onApply}>
          {isBusy ? 'Applying...' : 'Apply'}
        </Button>
        <Button variant="outline" size="sm" disabled={isBusy} onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  )
}

function VersionList({ versions }: { versions: number[] }) {
  const sorted = [...versions].sort((a, b) => b - a)
  if (sorted.length <= 2) {
    return <>v{sorted.join(', v')}</>
  }
  const shown = sorted.slice(0, 2)
  const remaining = sorted.length - 2
  return (
    <>
      v{shown.join(', v')} +{remaining} more
    </>
  )
}

function StatusBadge({ status }: { status: RowStatus }) {
  switch (status) {
    case 'active':
      return <Badge variant="default">Active</Badge>
    case 'missing':
      return <Badge variant="outline">Missing</Badge>
    case 'orphaned':
      return <Badge variant="destructive">Orphaned</Badge>
  }
}
