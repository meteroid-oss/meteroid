// Value pickers for the audience builder. Each one renders inline, opens on mount
// and hands a finished leaf to `onPick`. Country / currency / plan mirror the
// Explore cohort pickers so the two builders feel the same.

import {
  Button,
  Command,
  CommandEmpty,
  CommandInput,
  CommandItem,
  CommandList,
  Input,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@md/ui'
import { useMemo, useState } from 'react'

import { ValuesTagInput } from '@/components/form/ValuesTagInput'
import { CustomerMultiSelect } from '@/features/customers/CustomerMultiSelect'
import { getCountryFlagEmoji, getCountryName } from '@/features/settings/utils'
import { useQuery } from '@/lib/connectrpc'
import { majorToMinorUnit } from '@/lib/utils/numbers'
import { CustomPropertyDefinition, CustomPropertyType } from '@/rpc/api/customproperties/v1/models_pb'
import {
  getCountries,
  getCurrencies,
} from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

import {
  CMP_OP_LABEL,
  INVOICE_KIND_HELP,
  INVOICE_KIND_OPTIONS,
  PAYMENT_METHOD_TYPE_OPTIONS,
  PLAN_TYPE_OPTIONS,
  type AudienceLeaf,
  type CmpOp,
  type Option,
  type PropertyOp,
} from './types'

interface PickProps {
  onPick: (leaf: AudienceLeaf) => void
}

const triggerCls = 'h-7 px-2 text-xs'

// Popover + command list with a checkmark toggle and an "Add (n)" footer.
const MultiCheckPicker = ({
  options,
  placeholder,
  searchPlaceholder,
  help,
  onPick,
}: {
  options: Option[]
  placeholder: string
  searchPlaceholder: string
  help?: string
  onPick: (values: string[]) => void
}) => {
  const [open, setOpen] = useState(true)
  const [selected, setSelected] = useState<string[]>([])
  const toggle = (v: string) =>
    setSelected(prev => (prev.includes(v) ? prev.filter(x => x !== v) : [...prev, v]))

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" className={triggerCls} role="combobox">
          {selected.length > 0 ? `${selected.length} selected` : placeholder}
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[280px] p-0" align="start">
        <Command>
          <CommandInput placeholder={searchPlaceholder} className="h-9" />
          <CommandEmpty>Nothing found.</CommandEmpty>
          <CommandList>
            {options.map(o => (
              <CommandItem key={o.value} value={o.label} onSelect={() => toggle(o.value)}>
                <span className={selected.includes(o.value) ? 'font-medium' : ''}>
                  {selected.includes(o.value) ? '✓ ' : ''}
                  {o.label}
                </span>
              </CommandItem>
            ))}
          </CommandList>
        </Command>
        {help && <p className="border-t border-border px-3 py-2 text-[11px] leading-snug text-muted-foreground">{help}</p>}
        <div className="border-t border-border p-1.5">
          <Button
            variant="secondary"
            size="sm"
            className="h-7 w-full text-xs"
            disabled={selected.length === 0}
            onClick={() => selected.length > 0 && onPick(selected)}
          >
            Add {selected.length > 0 ? `(${selected.length})` : ''}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  )
}

export const PlanTypePicker = ({ onPick }: PickProps) => (
  <MultiCheckPicker
    options={PLAN_TYPE_OPTIONS}
    placeholder="Select plan type…"
    searchPlaceholder="Search plan type…"
    onPick={values => onPick({ attribute: { key: 'plan_type', values } })}
  />
)

export const InvoiceKindPicker = ({ onPick }: PickProps) => (
  <MultiCheckPicker
    options={INVOICE_KIND_OPTIONS}
    placeholder="Select invoice kind…"
    searchPlaceholder="Search invoice kind…"
    help={INVOICE_KIND_HELP}
    onPick={values => onPick({ attribute: { key: 'invoice_kind', values } })}
  />
)

export const PaymentMethodTypePicker = ({ onPick }: PickProps) => (
  <MultiCheckPicker
    options={PAYMENT_METHOD_TYPE_OPTIONS}
    placeholder="Select payment method…"
    searchPlaceholder="Search payment method…"
    onPick={values => onPick({ attribute: { key: 'payment_method_type', values } })}
  />
)

export const CountryPicker = ({ onPick }: PickProps) => {
  const countries = useQuery(getCountries).data?.countries
  const options = useMemo(
    () =>
      (countries ?? []).map(c => ({
        value: c.code,
        label: `${getCountryFlagEmoji(c.code)} ${getCountryName(c.code)}`,
      })),
    [countries]
  )
  return (
    <MultiCheckPicker
      options={options}
      placeholder="Select country…"
      searchPlaceholder="Search country…"
      onPick={values => onPick({ attribute: { key: 'country', values } })}
    />
  )
}

export const CurrencyPicker = ({ onPick }: PickProps) => {
  const [open, setOpen] = useState(true)
  const currencies = useQuery(getCurrencies).data?.currencies ?? []

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" className={triggerCls} role="combobox">
          Select currency…
        </Button>
      </PopoverTrigger>
      <PopoverContent className="w-[240px] p-0" align="start">
        <Command>
          <CommandInput placeholder="Search currency…" className="h-9" />
          <CommandEmpty>No currency found.</CommandEmpty>
          <CommandList>
            {currencies.map(currency => (
              <CommandItem
                key={currency.code}
                value={currency.code}
                keywords={[currency.name]}
                onSelect={() => {
                  onPick({ attribute: { key: 'currency', values: [currency.code] } })
                  setOpen(false)
                }}
              >
                <span className="font-medium pr-2">{currency.code}</span>
                <span className="text-muted-foreground truncate">{currency.name}</span>
              </CommandItem>
            ))}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  )
}

export const PlanPicker = ({ plans, onPick }: PickProps & { plans: { id: string; name: string }[] }) => (
  <MultiCheckPicker
    options={plans.map(p => ({ value: p.id, label: p.name }))}
    placeholder="Select plans…"
    searchPlaceholder="Search plan…"
    onPick={plan_ids => onPick({ plan: { plan_ids } })}
  />
)

export const CustomerPicker = ({ onPick }: PickProps) => {
  const [ids, setIds] = useState<string[]>([])
  return (
    <span className="inline-flex items-center gap-1">
      <CustomerMultiSelect value={ids} onChange={setIds} placeholder="Select customers…" className="h-7 min-w-[200px] text-xs" />
      <Button
        variant="secondary"
        size="sm"
        className={triggerCls}
        disabled={ids.length === 0}
        onClick={() => ids.length > 0 && onPick({ customer: { customer_ids: ids } })}
      >
        Add {ids.length > 0 ? `(${ids.length})` : ''}
      </Button>
    </span>
  )
}

const CmpOpSelect = ({ value, onChange }: { value: CmpOp; onChange: (op: CmpOp) => void }) => (
  <Select value={value} onValueChange={v => onChange(v as CmpOp)}>
    <SelectTrigger className="h-7 w-[56px] text-xs">
      <SelectValue />
    </SelectTrigger>
    <SelectContent>
      {(Object.keys(CMP_OP_LABEL) as CmpOp[]).map(o => (
        <SelectItem key={o} value={o}>
          {CMP_OP_LABEL[o]}
        </SelectItem>
      ))}
    </SelectContent>
  </Select>
)

/** Amount due, typed in major units and stored in minor units of the chosen currency. */
export const AmountPicker = ({ onPick, defaultCurrency }: PickProps & { defaultCurrency?: string }) => {
  const currencies = useQuery(getCurrencies).data?.currencies ?? []
  const [op, setOp] = useState<CmpOp>('gte')
  const [amount, setAmount] = useState('')
  const [currency, setCurrency] = useState(defaultCurrency ?? '')

  const num = Number(amount)
  const valid = amount.trim() !== '' && Number.isFinite(num) && num >= 0 && currency !== ''

  return (
    <span className="inline-flex items-center gap-1">
      <span className="text-xs text-muted-foreground">Amount due</span>
      <CmpOpSelect value={op} onChange={setOp} />
      <Input
        className="h-7 w-[100px] text-xs"
        type="number"
        min={0}
        step="any"
        placeholder="amount"
        value={amount}
        onChange={e => setAmount(e.target.value)}
      />
      <Select value={currency} onValueChange={setCurrency}>
        <SelectTrigger className="h-7 w-[90px] text-xs">
          <SelectValue placeholder="Currency" />
        </SelectTrigger>
        <SelectContent>
          {currencies.map(c => (
            <SelectItem key={c.code} value={c.code}>
              {c.code}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Button
        variant="secondary"
        size="sm"
        className={triggerCls}
        disabled={!valid}
        onClick={() =>
          valid && onPick({ amount: { op, threshold: Number(majorToMinorUnit(num, currency)), currency } })
        }
      >
        Add
      </Button>
    </span>
  )
}

const SET_OPS: { value: PropertyOp; label: string }[] = [
  { value: 'set', label: 'is set' },
  { value: 'not_set', label: 'is not set' },
]

// Which value widget a property type gets, besides the set / not-set ops every type has.
type Widget = 'text' | 'single' | 'multi' | 'boolean' | 'number' | 'none'
const widgetFor = (type: CustomPropertyType): Widget => {
  switch (type) {
    case CustomPropertyType.TEXT:
    case CustomPropertyType.URL:
    case CustomPropertyType.EMAIL:
      return 'text'
    case CustomPropertyType.SINGLE_SELECT:
      return 'single'
    case CustomPropertyType.MULTI_SELECT:
      return 'multi'
    case CustomPropertyType.BOOLEAN:
      return 'boolean'
    case CustomPropertyType.NUMBER:
      return 'number'
    default:
      return 'none'
  }
}

const valueOp = (widget: Widget): PropertyOp | null =>
  widget === 'text' || widget === 'single' ? 'in' : widget === 'multi' ? 'any' : widget === 'boolean' ? 'eq' : widget === 'number' ? 'gte' : null

/** Custom customer property: the widget follows the definition's type. */
export const CustomPropertyPicker = ({
  definitions,
  onPick,
}: PickProps & { definitions: CustomPropertyDefinition[] }) => {
  const [key, setKey] = useState(definitions[0]?.key ?? '')
  const def = definitions.find(d => d.key === key)
  const widget = def ? widgetFor(def.propertyType) : 'none'
  const [op, setOp] = useState<PropertyOp>(() => valueOp(widget) ?? 'set')
  const [values, setValues] = useState<string[]>([])
  const [text, setText] = useState('')
  const [bool, setBool] = useState('true')

  const pickKey = (k: string) => {
    const next = definitions.find(d => d.key === k)
    setKey(k)
    setOp((next && valueOp(widgetFor(next.propertyType))) ?? 'set')
    setValues([])
    setText('')
  }

  const options = def?.config?.options.map(o => ({ value: o.value, label: o.label ?? o.value })) ?? []
  const isSetOp = op === 'set' || op === 'not_set'
  // Select-style types (and text with configured options) pick from the option list,
  // which carries its own Add button.
  const usesCheckPicker =
    !isSetOp && (widget === 'single' || widget === 'multi' || (widget === 'text' && options.length > 0))
  const num = Number(text)
  const valid =
    !!def &&
    (isSetOp ||
      widget === 'boolean' ||
      (widget === 'text' && values.length > 0) ||
      (widget === 'number' && text.trim() !== '' && Number.isFinite(num)))

  const leaf = (): AudienceLeaf => {
    if (isSetOp) return { property: { key, op } }
    if (widget === 'boolean') return { property: { key, op: 'eq', value: bool === 'true' } }
    if (widget === 'number') return { property: { key, op, value: num } }
    return { property: { key, op, values } }
  }

  const opChoices: { value: PropertyOp; label: string }[] =
    widget === 'number'
      ? [...(Object.keys(CMP_OP_LABEL) as CmpOp[]).map(o => ({ value: o, label: CMP_OP_LABEL[o] })), ...SET_OPS]
      : widget === 'text' || widget === 'single'
        ? [{ value: 'in', label: 'in' }, ...SET_OPS]
        : widget === 'multi'
          ? [{ value: 'any', label: 'has any of' }, ...SET_OPS]
          : widget === 'boolean'
            ? [{ value: 'eq', label: 'is' }, ...SET_OPS]
            : SET_OPS

  if (definitions.length === 0) {
    return <span className="text-xs text-muted-foreground">No customer properties defined yet.</span>
  }

  return (
    <span className="inline-flex flex-wrap items-center gap-1">
      <Select value={key} onValueChange={pickKey}>
        <SelectTrigger className="h-7 w-[160px] text-xs">
          <SelectValue placeholder="Property" />
        </SelectTrigger>
        <SelectContent>
          {definitions.map(d => (
            <SelectItem key={d.key} value={d.key}>
              {d.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Select value={op} onValueChange={v => setOp(v as PropertyOp)}>
        <SelectTrigger className="h-7 w-auto min-w-[64px] gap-1 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {opChoices.map(o => (
            <SelectItem key={o.value} value={o.value}>
              {o.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {usesCheckPicker && (
        <MultiCheckPicker
          options={options}
          placeholder="Select values…"
          searchPlaceholder="Search values…"
          onPick={picked => onPick({ property: { key, op, values: picked } })}
        />
      )}
      {!isSetOp && widget === 'text' && options.length === 0 && (
        <span className="min-w-[200px]">
          <ValuesTagInput value={values} onChange={setValues} placeholder="Value, Enter to add" />
        </span>
      )}
      {!isSetOp && widget === 'boolean' && (
        <Select value={bool} onValueChange={setBool}>
          <SelectTrigger className="h-7 w-[80px] text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="true">true</SelectItem>
            <SelectItem value="false">false</SelectItem>
          </SelectContent>
        </Select>
      )}
      {!isSetOp && widget === 'number' && (
        <Input
          className="h-7 w-[90px] text-xs"
          type="number"
          step="any"
          placeholder="value"
          value={text}
          onChange={e => setText(e.target.value)}
        />
      )}
      {!usesCheckPicker && (
        <Button variant="secondary" size="sm" className={triggerCls} disabled={!valid} onClick={() => valid && onPick(leaf())}>
          Add
        </Button>
      )}
    </span>
  )
}
