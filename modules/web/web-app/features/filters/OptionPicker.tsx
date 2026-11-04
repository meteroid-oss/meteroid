import { Command, CommandEmpty, CommandInput, CommandItem, CommandList } from '@md/ui'
import { cn } from '@ui/lib'
import { Check } from 'lucide-react'
import { ReactNode, useMemo, useState } from 'react'

import { useMembers } from '@/features/accounts/hooks/useMembers'
import { getCountryFlagEmoji } from '@/features/settings/utils'
import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import {
  listCustomerCountries,
  listCustomers,
  listCustomersByIds,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import {
  getCountries,
  getCurrencies,
} from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listTenantCurrencies } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

import { FieldWidget, MEMBER_ME } from './filterTypes'

/** Widgets whose values are picked from a (searchable) list rather than typed. */
export type OptionWidget = Extract<
  FieldWidget,
  'country' | 'currency' | 'customer' | 'plan' | 'member'
>

export const OPTION_WIDGETS: ReadonlySet<FieldWidget> = new Set<FieldWidget>([
  'country',
  'currency',
  'customer',
  'plan',
  'member',
])

interface PickerOption {
  value: string
  label: string
  /** Secondary text (e.g. the currency name next to its code). */
  hint?: string
  icon?: ReactNode
}

interface Source {
  /** Options for the current search; with no search, suggestions (in-use values first). */
  options: PickerOption[]
  /** Label for any value, including selected ones outside `options`. */
  labelOf: (value: string) => string
  isLoading: boolean
}

const MAX_SUGGESTED = 10
const MAX_RESULTS = 30

const matches = (q: string, ...texts: (string | undefined)[]) =>
  texts.some(t => t?.toLowerCase().includes(q))

/** Selected values first, then suggestions, then the rest of `all`, capped. */
function suggest(all: PickerOption[], selected: string[], inUse: string[]): PickerOption[] {
  const byValue = new Map(all.map(o => [o.value, o]))
  const seen = new Set<string>()
  const out: PickerOption[] = []
  const push = (o?: PickerOption) => {
    if (o && !seen.has(o.value)) {
      seen.add(o.value)
      out.push(o)
    }
  }
  selected.forEach(v => push(byValue.get(v) ?? { value: v, label: v }))
  inUse.forEach(v => push(byValue.get(v)))
  for (const o of all) {
    if (out.length >= Math.max(MAX_SUGGESTED, selected.length)) break
    push(o)
  }
  return out
}

function useCountrySource(search: string, selected: string[]): Source {
  const countries = useQuery(getCountries)
  const inUse = useQuery(listCustomerCountries, {}).data?.countries ?? []
  const all = useMemo(
    () =>
      (countries.data?.countries ?? []).map(c => ({
        value: c.code,
        label: c.name,
        icon: <span>{getCountryFlagEmoji(c.code)}</span>,
      })),
    [countries.data]
  )
  const q = search.trim().toLowerCase()
  const options = q
    ? all.filter(o => matches(q, o.label, o.value)).slice(0, MAX_RESULTS)
    : suggest(all, selected, inUse)
  const byValue = new Map(all.map(o => [o.value, o.label]))
  return { options, labelOf: v => byValue.get(v) ?? v, isLoading: countries.isLoading }
}

function useCurrencySource(search: string, selected: string[]): Source {
  const currencies = useQuery(getCurrencies)
  const inUse = useQuery(listTenantCurrencies).data?.currencies ?? []
  const all = useMemo(
    () =>
      (currencies.data?.currencies ?? []).map(c => ({
        value: c.code,
        label: c.code,
        hint: c.name,
      })),
    [currencies.data]
  )
  const q = search.trim().toLowerCase()
  const options = q
    ? all.filter(o => matches(q, o.value, o.hint)).slice(0, MAX_RESULTS)
    : suggest(all, selected, inUse)
  return { options, labelOf: v => v, isLoading: currencies.isLoading }
}

function usePlanSource(search: string, selected: string[]): Source {
  const plans = useQuery(listPlans, { pagination: { perPage: 200, page: 0 } })
  const all = useMemo(
    () => (plans.data?.plans ?? []).map(p => ({ value: p.id, label: p.name })),
    [plans.data]
  )
  const q = search.trim().toLowerCase()
  const options = q
    ? all.filter(o => matches(q, o.label)).slice(0, MAX_RESULTS)
    : suggest(all, selected, [])
  const byValue = new Map(all.map(o => [o.value, o.label]))
  return { options, labelOf: v => byValue.get(v) ?? 'Unknown plan', isLoading: plans.isLoading }
}

/** Names for the given customer ids (chips and selected rows outside the search results). */
function useCustomerNames(ids: string[]) {
  const names = useQuery(listCustomersByIds, { ids }, { enabled: ids.length > 0 })
  return useMemo(
    () => new Map((names.data?.customers ?? []).map(c => [c.id, c.name])),
    [names.data]
  )
}

function useCustomerSource(search: string, selected: string[]): Source {
  const debounced = useDebounceValue(search.trim(), 250)
  const results = useQuery(listCustomers, {
    search: debounced || undefined,
    pagination: { perPage: MAX_RESULTS, page: 0 },
  })
  const names = useCustomerNames(selected)
  const found = (results.data?.customers ?? []).map(c => ({
    value: c.id,
    label: c.name,
    hint: c.alias ?? c.billingEmail,
  }))
  const options = debounced
    ? found
    : suggest(
        [...selected.map(id => ({ value: id, label: names.get(id) ?? '…' })), ...found],
        selected,
        []
      )
  return {
    options,
    labelOf: v => names.get(v) ?? found.find(o => o.value === v)?.label ?? '…',
    isLoading: results.isLoading,
  }
}

function useMemberSource(search: string, selected: string[]): Source {
  const { members, byId, isLoading } = useMembers()
  const all = useMemo<PickerOption[]>(
    () => [
      { value: MEMBER_ME, label: 'Me' },
      ...members.map(m => ({ value: m.id, label: m.name, hint: m.email })),
    ],
    [members]
  )
  const q = search.trim().toLowerCase()
  const options = q
    ? all.filter(o => matches(q, o.label, o.hint)).slice(0, MAX_RESULTS)
    : suggest(all, selected, [])
  const labelOf = (v: string) =>
    v === MEMBER_ME ? 'Me' : (byId.get(v)?.name ?? (isLoading ? '…' : 'Former member'))
  return { options, labelOf, isLoading }
}

/** Source hook per widget. Pickers are keyed by widget, so each instance always calls the same one. */
const SOURCE_HOOKS: Record<OptionWidget, (search: string, selected: string[]) => Source> = {
  country: useCountrySource,
  currency: useCurrencySource,
  plan: usePlanSource,
  customer: useCustomerSource,
  member: useMemberSource,
}

const PLACEHOLDER: Record<OptionWidget, string> = {
  country: 'Search countries…',
  currency: 'Search currencies…',
  plan: 'Search plans…',
  customer: 'Search customers…',
  member: 'Search members…',
}

interface PickerProps {
  widget: OptionWidget
  values: string[]
  /** Several values (`is any of`) or exactly one (`is` / `is not`). */
  multi: boolean
  onChange: (values: string[]) => void
}

/**
 * Searchable value picker for list-backed fields (country, currency, plan, customer). With an
 * empty search it suggests the selection and the values actually in use; typing searches the full
 * list (customers are searched server-side).
 */
export const OptionPicker = (props: PickerProps) => (
  // Keyed by widget so switching fields remounts (hooks differ per source).
  <OptionPickerInner key={props.widget} {...props} />
)

const OptionPickerInner = ({ widget, values, multi, onChange }: PickerProps) => {
  const [search, setSearch] = useState('')
  const useWidgetSource = SOURCE_HOOKS[widget]
  const source = useWidgetSource(search, values)
  const selected = new Set(values)

  const pick = (value: string) => {
    if (!multi) return onChange(selected.has(value) ? [] : [value])
    onChange(selected.has(value) ? values.filter(v => v !== value) : [...values, value])
  }

  return (
    <Command shouldFilter={false} className="rounded-md border">
      <CommandInput
        autoFocus
        placeholder={PLACEHOLDER[widget]}
        value={search}
        onValueChange={setSearch}
      />
      <CommandList className="max-h-56">
        <CommandEmpty>{source.isLoading ? 'Loading…' : 'Nothing found.'}</CommandEmpty>
        {source.options.map(o => (
          <CommandItem
            key={o.value}
            value={o.value}
            onSelect={() => pick(o.value)}
            className="gap-2 text-xs"
          >
            <Check
              className={cn(
                'h-3.5 w-3.5 shrink-0',
                selected.has(o.value) ? 'opacity-100' : 'opacity-0'
              )}
            />
            {o.icon}
            <span className="truncate">{o.label}</span>
            {o.hint && <span className="ml-auto truncate text-muted-foreground">{o.hint}</span>}
          </CommandItem>
        ))}
      </CommandList>
    </Command>
  )
}

/** Compact, human-readable summary of picked values for a filter chip. */
export const OptionSummary = ({ widget, values }: { widget: OptionWidget; values: string[] }) => (
  <OptionSummaryInner key={widget} widget={widget} values={values} />
)

const OptionSummaryInner = ({ widget, values }: { widget: OptionWidget; values: string[] }) => {
  const useWidgetSource = SOURCE_HOOKS[widget]
  const source = useWidgetSource('', values)
  const labels = values.map(source.labelOf)
  const shown = labels.slice(0, 2).join(', ')
  return <>{labels.length > 2 ? `${shown} +${labels.length - 2}` : shown}</>
}
