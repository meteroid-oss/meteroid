import { cn } from '@md/ui'

/**
 * A row's trailing actions column (the "⋯" menu): `id: 'actions'`, or an unlabelled column on
 * `accessorKey: 'id'`. It shrinks to its button and pins it to the right edge of the row, so it
 * doesn't take a share of the free width and float somewhere in the middle.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const isActionColumn = (colDef: any): boolean =>
  colDef?.id === 'actions' || (colDef?.accessorKey === 'id' && colDef?.header === '')

const actionColumnClassName = 'w-px whitespace-nowrap text-right'

/** Id of the row-selection column that CustomTable puts first. */
export const SELECT_COLUMN_ID = '__select__'

/**
 * The selection checkbox column shrinks to the checkbox too, and keeps 24px between the checkbox
 * and the first real column: 14px here plus that column's own 10px left padding. (`!`: the table
 * sets cell padding through a descendant selector, which outranks a class on the cell.)
 */
const selectColumnClassName = 'w-px !pr-3.5'

/** The column's own `className`, plus the layout of the actions or selection column. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export const columnClassName = (colDef: any): string | undefined => {
  const own: string | undefined = colDef?.className
  if (colDef?.id === SELECT_COLUMN_ID) return cn(own, selectColumnClassName)
  return isActionColumn(colDef) ? cn(own, actionColumnClassName) : own
}
