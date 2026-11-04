import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'

import { FilterValueInput } from './FilterValueInput'
import { FilterOp, OP_LABELS } from './filterTypes'
import { ResolvedField } from './useFilterFields'

interface Props {
  field: ResolvedField
  op: FilterOp
  values: string[]
  onOpChange: (op: FilterOp) => void
  onValuesChange: (values: string[]) => void
}

/** Operator selector + type-aware value input for one field. */
export const FilterEditor = ({ field, op, values, onOpChange, onValuesChange }: Props) => (
  <div className="flex flex-col gap-2">
    <Select value={op} onValueChange={v => onOpChange(v as FilterOp)}>
      <SelectTrigger className="h-8">
        <SelectValue />
      </SelectTrigger>
      <SelectContent>
        {field.ops.map(o => (
          <SelectItem key={o} value={o}>
            {OP_LABELS[o]}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
    <FilterValueInput field={field} op={op} values={values} onChange={onValuesChange} autoFocus />
  </div>
)
