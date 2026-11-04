import { Badge, Button, Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@md/ui'
import { PlusIcon } from '@radix-ui/react-icons'
import { useMemo, useState } from 'react'

import { RuleBuilder } from '@/features/rules/RuleChrome'
import { parseModel, serializeModel } from '@/features/rules/ruleModel'

import { groupDepth, leafLabel, MAX_GROUP_DEPTH, sanitizeAudience } from './model'
import {
  AmountPicker,
  CountryPicker,
  CurrencyPicker,
  CustomerPicker,
  CustomPropertyPicker,
  InvoiceKindPicker,
  PaymentMethodTypePicker,
  PlanPicker,
  PlanTypePicker,
} from './pickers'
import { AUDIENCE_ROOT_HELP, type AudienceLeaf, type AudiencePredicate } from './types'
import { useAudienceNames } from './useAudienceNames'

interface Props {
  predicates: AudiencePredicate[]
  onChange: (predicates: AudiencePredicate[]) => void
  /** Pre-selected currency of the amount picker (the entity's, typically). */
  defaultCurrency?: string
  /** Offer only customer, country and customer property conditions (no invoice to match on). */
  customerLevelOnly?: boolean
  emptyLabel?: string
  help?: string
}

type DraftKind =
  | 'plan_type'
  | 'invoice_kind'
  | 'payment_method_type'
  | 'currency'
  | 'country'
  | 'plan'
  | 'customer'
  | 'amount'
  | 'property'

const DRAFT_OPTIONS: { value: DraftKind; label: string; customerLevel: boolean }[] = [
  { value: 'plan_type', label: 'Plan type', customerLevel: false },
  { value: 'plan', label: 'Plan', customerLevel: false },
  { value: 'invoice_kind', label: 'Invoice kind', customerLevel: false },
  { value: 'payment_method_type', label: 'Payment method', customerLevel: false },
  { value: 'amount', label: 'Amount due', customerLevel: false },
  { value: 'currency', label: 'Currency', customerLevel: false },
  { value: 'country', label: 'Country', customerLevel: true },
  { value: 'customer', label: 'Customer', customerLevel: true },
  { value: 'property', label: 'Customer property', customerLevel: true },
]

const AddLeaf = ({
  plans,
  definitions,
  defaultCurrency,
  customerLevelOnly,
  onAdd,
}: {
  plans: { id: string; name: string }[]
  definitions: Parameters<typeof CustomPropertyPicker>[0]['definitions']
  defaultCurrency?: string
  customerLevelOnly?: boolean
  onAdd: (leaf: AudienceLeaf) => void
}) => {
  const [draft, setDraft] = useState<DraftKind | null>(null)
  const done = (leaf: AudienceLeaf) => {
    onAdd(leaf)
    setDraft(null)
  }
  if (draft) {
    return (
      <span className="inline-flex flex-wrap items-center gap-1">
        {draft === 'plan_type' ? (
          <PlanTypePicker onPick={done} />
        ) : draft === 'invoice_kind' ? (
          <InvoiceKindPicker onPick={done} />
        ) : draft === 'payment_method_type' ? (
          <PaymentMethodTypePicker onPick={done} />
        ) : draft === 'currency' ? (
          <CurrencyPicker onPick={done} />
        ) : draft === 'country' ? (
          <CountryPicker onPick={done} />
        ) : draft === 'plan' ? (
          <PlanPicker plans={plans} onPick={done} />
        ) : draft === 'customer' ? (
          <CustomerPicker onPick={done} />
        ) : draft === 'amount' ? (
          <AmountPicker onPick={done} defaultCurrency={defaultCurrency} />
        ) : (
          <CustomPropertyPicker definitions={definitions} onPick={done} />
        )}
        <Button variant="ghost" size="sm" className="h-7 px-1.5 text-xs" onClick={() => setDraft(null)}>
          Cancel
        </Button>
      </span>
    )
  }
  return (
    <Select onValueChange={v => setDraft(v as DraftKind)} value="">
      <SelectTrigger className="h-7 w-auto gap-1 px-2 text-xs">
        <PlusIcon className="h-3 w-3" />
        <SelectValue placeholder="Add condition" />
      </SelectTrigger>
      <SelectContent>
        {DRAFT_OPTIONS.filter(o => !customerLevelOnly || o.customerLevel).map(o => (
          <SelectItem key={o.value} value={o.value}>
            {o.label}
          </SelectItem>
        ))}
        <SelectItem value="segment" disabled>
          Saved segment — coming soon
        </SelectItem>
      </SelectContent>
    </Select>
  )
}

export const AudienceBuilder = ({
  predicates,
  onChange,
  defaultCurrency,
  customerLevelOnly,
  emptyLabel = 'every open invoice',
  help = AUDIENCE_ROOT_HELP,
}: Props) => {
  const { plans, definitions, ctx } = useAudienceNames(predicates)
  // Archived definitions are kept out of the picker but still label (and flag)
  // the leaves that already reference them.
  const activeDefinitions = definitions.filter(d => !d.archived)
  const archivedKeys = useMemo(() => new Set(definitions.filter(d => d.archived).map(d => d.key)), [definitions])

  const model = parseModel<AudienceLeaf>(predicates)
  // Same check as the server (after collapsing single-child groups), shown while editing.
  const tooDeep = sanitizeAudience(predicates).some(p => groupDepth(p) > MAX_GROUP_DEPTH)

  return (
    <div className="flex flex-col gap-2">
      <RuleBuilder
        model={model}
        onChange={m => onChange(serializeModel(m))}
        renderLeaf={leaf => (
          <>
            {leafLabel(leaf, ctx)}
            {'property' in leaf && archivedKeys.has(leaf.property.key) && (
              <Badge variant="warning" size="sm" className="ml-1.5 font-normal">
                Archived property
              </Badge>
            )}
          </>
        )}
        renderAdd={onAdd => (
          <AddLeaf
            plans={plans}
            definitions={activeDefinitions}
            defaultCurrency={defaultCurrency}
            customerLevelOnly={customerLevelOnly}
            onAdd={onAdd}
          />
        )}
        emptyLabel={emptyLabel}
      />
      {tooDeep && (
        <p className="text-[13px] text-destructive">
          Too deeply nested: groups and NOT combine at most {MAX_GROUP_DEPTH} levels deep. Remove a NOT
          or move a condition out of its group.
        </p>
      )}
      {help && <p className="text-[13px] text-muted-foreground">{help}</p>}
    </div>
  )
}
