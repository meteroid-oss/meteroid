import { CustomPropertyType } from '@/rpc/api/customproperties/v1/models_pb'

/** App-side filter operator (mirrors the backend `FilterOperator`). */
export type FilterOp =
  | 'equals'
  | 'not_equals'
  | 'in'
  | 'contains'
  | 'gt'
  | 'gte'
  | 'lt'
  | 'lte'
  | 'between'
  | 'is_set'
  | 'is_empty'
  | 'is_true'
  | 'is_false'
  | 'before'
  | 'after'

/** A single filter as edited in the UI. `field` is a native column id or `cp:<key>`. */
export interface AppFilter {
  field: string
  op: FilterOp
  values: string[]
}

export const OP_LABELS: Record<FilterOp, string> = {
  equals: 'is',
  not_equals: 'is not',
  in: 'is any of',
  contains: 'contains',
  gt: '>',
  gte: '≥',
  lt: '<',
  lte: '≤',
  between: 'between',
  is_set: 'is set',
  is_empty: 'is empty',
  is_true: 'is true',
  is_false: 'is false',
  before: 'before',
  after: 'after',
}

/** Operators that take no value input. */
export const VALUELESS_OPS: ReadonlySet<FilterOp> = new Set<FilterOp>([
  'is_set',
  'is_empty',
  'is_true',
  'is_false',
])

/** Operators that take multiple values (a token/multi-select input). */
export const MULTI_VALUE_OPS: ReadonlySet<FilterOp> = new Set<FilterOp>(['in', 'between'])

/** Allowed operators for a custom-property type — mirrors `FilterOperator::allowed_for` server-side. */
export function allowedOpsForPropertyType(type: CustomPropertyType): FilterOp[] {
  switch (type) {
    case CustomPropertyType.TEXT:
    case CustomPropertyType.URL:
    case CustomPropertyType.EMAIL:
      return ['equals', 'not_equals', 'in', 'contains', 'is_set', 'is_empty']
    case CustomPropertyType.NUMBER:
      return [
        'equals',
        'not_equals',
        'in',
        'gt',
        'gte',
        'lt',
        'lte',
        'between',
        'is_set',
        'is_empty',
      ]
    case CustomPropertyType.BOOLEAN:
      return ['is_true', 'is_false', 'is_set']
    case CustomPropertyType.DATE:
    case CustomPropertyType.DATETIME:
      return ['equals', 'before', 'after', 'between', 'is_set', 'is_empty']
    case CustomPropertyType.SINGLE_SELECT:
      return ['equals', 'not_equals', 'in', 'is_set', 'is_empty']
    case CustomPropertyType.MULTI_SELECT:
      return ['contains', 'in', 'is_set', 'is_empty']
    default:
      return ['is_set', 'is_empty']
  }
}

/** The value-input widget a field uses. */
export type FieldWidget =
  | 'text'
  | 'number'
  | 'boolean'
  | 'date'
  | 'datetime'
  | 'select'
  /** Searchable pickers (see `OptionPicker`). */
  | 'country'
  | 'currency'
  | 'customer'
  | 'plan'
  /** An organization member; {@link MEMBER_ME} stands for the caller. */
  | 'member'
  /** An amount in major units (e.g. 100.50); the server scales it per row currency. */
  | 'money'
  /** Presence only (`is set` / `is empty`). */
  | 'presence'

/** A native (non-custom-property) filterable field. */
export interface NativeField {
  /** Column id sent to the API (no `cp:` prefix). */
  id: string
  label: string
  widget: FieldWidget
  ops: FilterOp[]
  /** Static options for `select` widgets. */
  options?: { value: string; label: string }[]
}

/** The entity lists that support the filter bar. */
export type NativeFieldEntity = 'customer' | 'subscription' | 'invoice' | 'credit_note' | 'quote'

/** Symbolic member value resolved to the caller's user id when sent, so a shared view means "me". */
export const MEMBER_ME = 'me'

// Operator sets per native widget — mirror `native_ops` in the store's `filter_resolution.rs`.
const PICK_OPS: FilterOp[] = ['in', 'not_equals']
const DATE_OPS: FilterOp[] = ['before', 'after', 'between']
const MONEY_OPS: FilterOp[] = ['gt', 'lt', 'between', 'equals', 'gte', 'lte']
const PRESENCE_OPS: FilterOp[] = ['is_set', 'is_empty']

const currency: NativeField = {
  id: 'currency',
  label: 'Currency',
  widget: 'currency',
  ops: PICK_OPS,
}
const customer: NativeField = {
  id: 'customer',
  label: 'Customer',
  widget: 'customer',
  ops: PICK_OPS,
}
const plan: NativeField = { id: 'plan', label: 'Plan', widget: 'plan', ops: ['in'] }
const money = (id: string, label: string): NativeField => ({
  id,
  label,
  widget: 'money',
  ops: MONEY_OPS,
})
const date = (id: string, label: string, nullable = false): NativeField => ({
  id,
  label,
  widget: 'date',
  ops: nullable ? [...DATE_OPS, ...PRESENCE_OPS] : DATE_OPS,
})

/**
 * Built-in filterable fields per entity list. Must match the server registry (`native_fields` in
 * the store's `filter_resolution.rs`): an unknown field or operator is rejected there.
 */
export const NATIVE_FIELDS: Record<NativeFieldEntity, NativeField[]> = {
  customer: [
    { id: 'country', label: 'Country', widget: 'country', ops: [...PICK_OPS, ...PRESENCE_OPS] },
    currency,
    date('created_at', 'Created'),
    money('balance', 'Balance'),
    { id: 'billing_email', label: 'Billing email', widget: 'presence', ops: PRESENCE_OPS },
    { id: 'owner', label: 'Owner', widget: 'member', ops: [...PICK_OPS, ...PRESENCE_OPS] },
  ],
  subscription: [
    plan,
    customer,
    currency,
    money('mrr', 'MRR'),
    date('start_date', 'Start date'),
    date('end_date', 'End date', true),
  ],
  invoice: [
    customer,
    currency,
    money('total', 'Total'),
    money('amount_due', 'Amount due'),
    date('invoice_date', 'Invoice date'),
    date('due_at', 'Due date', true),
  ],
  credit_note: [
    customer,
    currency,
    money('total', 'Total'),
    date('created_at', 'Created'),
    {
      id: 'credit_type',
      label: 'Credit type',
      widget: 'select',
      ops: PICK_OPS,
      options: [
        { value: 'CREDIT_TO_BALANCE', label: 'Credit to balance' },
        { value: 'REFUND', label: 'Refund' },
        { value: 'DEBT_CANCELLATION', label: 'Debt cancellation' },
      ],
    },
  ],
  quote: [
    customer,
    plan,
    currency,
    date('created_at', 'Created'),
    date('expires_at', 'Expires', true),
  ],
}

/**
 * Values to keep when the operator changes: all of them between multi-value pickers and single ones
 * (trimmed to one), none across different arities (`between`) or into a valueless operator.
 */
export function carryValues(from: FilterOp, to: FilterOp, values: string[]): string[] {
  if (VALUELESS_OPS.has(to) || VALUELESS_OPS.has(from)) return []
  if ((from === 'between') !== (to === 'between')) return []
  return MULTI_VALUE_OPS.has(to) ? values : values.slice(0, 1)
}
