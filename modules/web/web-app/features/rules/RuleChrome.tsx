// Shared chrome for rule builders: the ALL/ANY root, negatable condition chips and
// dashed groups. Leaf-agnostic — the builder renders its own labels and pickers.

import { Button, Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { Cross2Icon, PlusIcon } from '@radix-ui/react-icons'
import { ReactNode } from 'react'

import type { Cond, Model, Row } from '@/features/rules/ruleModel'

export const NegateButton = ({ negate, onClick }: { negate: boolean; onClick: () => void }) => (
  <button
    type="button"
    onClick={onClick}
    title={negate ? 'Negated (NOT) — click to un-negate' : 'Negate (NOT)'}
    className={`rounded px-1 text-[11px] font-medium leading-none ${
      negate ? 'bg-destructive/15 text-destructive' : 'text-muted-foreground hover:text-foreground'
    }`}
  >
    ¬
  </button>
)

export const AllAny = ({
  value,
  onChange,
  className,
}: {
  value: 'and' | 'or'
  onChange: (v: 'and' | 'or') => void
  className?: string
}) => (
  <Select value={value} onValueChange={v => onChange(v as 'and' | 'or')}>
    <SelectTrigger className={className ?? 'h-7 w-[88px] text-xs'}>
      <SelectValue />
    </SelectTrigger>
    <SelectContent>
      <SelectItem value="and">ALL</SelectItem>
      <SelectItem value="or">ANY</SelectItem>
    </SelectContent>
  </Select>
)

export const CondChip = ({
  negate,
  onNegate,
  onRemove,
  children,
}: {
  negate: boolean
  onNegate: () => void
  onRemove: () => void
  children: ReactNode
}) => (
  <span className="inline-flex items-center gap-1 rounded-full border border-border bg-muted/40 py-1 pl-1 pr-2 text-xs">
    <NegateButton negate={negate} onClick={onNegate} />
    <span>{children}</span>
    <button onClick={onRemove} className="text-muted-foreground hover:text-foreground" aria-label="Remove condition">
      <Cross2Icon className="h-3 w-3" />
    </button>
  </span>
)

interface RuleBuilderProps<L> {
  model: Model<L>
  onChange: (model: Model<L>) => void
  renderLeaf: (leaf: L) => ReactNode
  /** The "+ add condition" control; `onAdd` appends the built leaf where it was rendered. */
  renderAdd: (onAdd: (leaf: L) => void) => ReactNode
  /** Shown next to the root selector when there is no condition yet. */
  emptyLabel: string
}

export const RuleBuilder = <L,>({ model, onChange, renderLeaf, renderAdd, emptyLabel }: RuleBuilderProps<L>) => {
  const setRows = (rows: Row<L>[]) => onChange({ ...model, rows })
  const updateRow = (i: number, r: Row<L>) => setRows(model.rows.map((x, idx) => (idx === i ? r : x)))
  const removeRow = (i: number) => setRows(model.rows.filter((_, idx) => idx !== i))

  const condChip = (cond: Cond<L>, onNegate: () => void, onRemove: () => void) => (
    <CondChip negate={cond.negate} onNegate={onNegate} onRemove={onRemove}>
      {renderLeaf(cond.leaf)}
    </CondChip>
  )

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center gap-1.5 text-xs">
        <span className="text-muted-foreground">Match</span>
        <AllAny value={model.op} onChange={op => onChange({ ...model, op })} />
        <span className="text-muted-foreground">of:</span>
        {model.rows.length === 0 && <span className="text-muted-foreground/70">{emptyLabel}</span>}
      </div>

      {model.rows.length > 0 && (
        <div className="flex flex-col gap-1.5 pl-1">
          {model.rows.map((row, i) =>
            row.t === 'cond' ? (
              <div key={i} className="flex flex-wrap items-center gap-1.5">
                {condChip(
                  row.cond,
                  () => updateRow(i, { t: 'cond', cond: { ...row.cond, negate: !row.cond.negate } }),
                  () => removeRow(i)
                )}
              </div>
            ) : (
              <div key={i} className="flex flex-col gap-1.5 rounded-md border border-dashed border-border p-2">
                <div className="flex items-center gap-1.5 text-xs">
                  <NegateButton
                    negate={row.group.negate}
                    onClick={() => updateRow(i, { t: 'group', group: { ...row.group, negate: !row.group.negate } })}
                  />
                  <span className="text-muted-foreground">Match</span>
                  <AllAny
                    value={row.group.op}
                    onChange={op => updateRow(i, { t: 'group', group: { ...row.group, op } })}
                    className="h-6 w-[80px] text-xs"
                  />
                  <span className="text-muted-foreground">of:</span>
                  <button
                    onClick={() => removeRow(i)}
                    className="ml-auto text-muted-foreground hover:text-foreground"
                    aria-label="Remove group"
                  >
                    <Cross2Icon className="h-3 w-3" />
                  </button>
                </div>
                <div className="flex flex-wrap items-center gap-1.5 pl-1">
                  {row.group.conds.map((c, j) => (
                    <span key={j}>
                      {condChip(
                        c,
                        () =>
                          updateRow(i, {
                            t: 'group',
                            group: {
                              ...row.group,
                              conds: row.group.conds.map((x, jdx) => (jdx === j ? { ...x, negate: !x.negate } : x)),
                            },
                          }),
                        () =>
                          updateRow(i, {
                            t: 'group',
                            group: { ...row.group, conds: row.group.conds.filter((_, jdx) => jdx !== j) },
                          })
                      )}
                    </span>
                  ))}
                  {renderAdd(leaf =>
                    updateRow(i, {
                      t: 'group',
                      group: { ...row.group, conds: [...row.group.conds, { negate: false, leaf }] },
                    })
                  )}
                </div>
              </div>
            )
          )}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-1.5 pl-1">
        {renderAdd(leaf => setRows([...model.rows, { t: 'cond', cond: { negate: false, leaf } }]))}
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-xs"
          onClick={() => setRows([...model.rows, { t: 'group', group: { negate: false, op: 'or', conds: [] } }])}
        >
          <PlusIcon className="mr-1 h-3 w-3" /> Add group
        </Button>
      </div>
    </div>
  )
}
