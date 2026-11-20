import {
  Badge,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  cn,
} from '@md/ui'
import { ArrowDown, ArrowUp, Minus, MoreHorizontal, Plus, X } from 'lucide-react'
import { useRef, useState } from 'react'

import { formDataToPrice } from '@/features/pricing'
import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { feeFigures } from '@/features/quotes/editor/lineFigures'
import { discountError } from '@/features/quotes/editor/quoteV2'
import {
  ComponentConfigurationFields,
  deriveFeeType,
  getComponentBillingLabel,
  getExampleQuantity,
  isComponentConfigured,
  planComponentDisplayPrice,
  planComponentSlotTotal,
  requiresConfiguration,
} from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import { getPriceBillingLabel, priceToSubscriptionFee } from '@/lib/mapping/priceToSubscriptionFee'
import { formatCurrencyNoRounding, formatUsage } from '@/utils/numbers'

import type { ComponentFeeType } from '@/features/pricing'
import type { LineFigures } from '@/features/quotes/editor/lineFigures'
import type { EditorLine, LineDetailState, QuoteV2State } from '@/features/quotes/editor/quoteV2'
import type {
  PriceComponentsState,
  UsageExampleKey,
} from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import type {
  ComponentParameterization,
  ExtraComponent,
} from '@/pages/tenants/subscription/create/state'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { Dispatch, ReactNode, SetStateAction } from 'react'

type Row =
  | { kind: 'component'; line: EditorLine; component: PriceComponent }
  | { kind: 'extra'; line: EditorLine; extra: ExtraComponent; index: number }
  | { kind: 'addOn'; line: EditorLine; addOnId: string; quantity: number; addOn?: AddOn }

interface RowCells {
  badge?: string
  cadence?: string
  quantity: ReactNode
  price: ReactNode
  amount: ReactNode
  menu: ReactNode
  needsConfiguration: boolean
}

const FEE_BADGES: Partial<Record<ComponentFeeType, string>> = {
  slot: 'Seat',
  usage: 'Usage',
  capacity: 'Capacity',
  credit: 'Credit',
  oneTime: 'One-time',
  extraRecurring: 'Recurring',
}

const isExampleUsage = (price: Price | undefined) =>
  price?.pricing.case === 'usagePricing' && price.pricing.value.model.case !== 'matrix'

const NONE = <span className="text-muted-foreground">—</span>

// Every cell's first line is this tall, so text and inputs share a baseline across the row.
const FirstLine = ({ className, children }: { className?: string; children: ReactNode }) => (
  <div className={cn('flex min-h-7 items-center', className)}>{children}</div>
)

export interface LinesTableActions {
  toggleRemoval: (componentId: string) => void
  updateConfiguration: (componentId: string, config: Partial<ComponentParameterization>) => void
  startOverride: (componentId: string) => void
  removeOverride: (componentId: string) => void
  editExtra: (index: number) => void
  removeExtra: (index: number) => void
  setExampleQuantity: (key: UsageExampleKey, quantity: string) => void
  setAddOnQuantity: (addOnId: string, quantity: number) => void
  removeAddOn: (addOnId: string) => void
}

interface LinesTableProps {
  currency: string
  planComponents: PriceComponent[]
  pricing: PriceComponentsState
  /** The priced lines, in the order the customer reads them. */
  lines: EditorLine[]
  addOns: { addOnId: string; quantity?: number }[]
  availableAddOns: AddOn[]
  exampleAmountByKey: Map<string, string | undefined>
  v2: QuoteV2State
  setV2: Dispatch<SetStateAction<QuoteV2State>>
  /** Descriptions, discounts and order; without it the lines read as priced. */
  lineEditing: boolean
  actions: LinesTableActions
}

export const LinesTable = ({
  currency,
  planComponents,
  pricing,
  lines,
  addOns,
  availableAddOns,
  exampleAmountByKey,
  v2,
  setV2,
  lineEditing,
  actions,
}: LinesTableProps) => {
  const creditUnits = useCreditUnits()
  const [editingDescription, setEditingDescription] = useState<string>()

  const usedExtras = new Set<number>()
  const rows: Row[] = lines.flatMap((line): Row[] => {
    const id = line.key.slice(2)
    switch (line.key.slice(0, 2)) {
      case 'c:': {
        const component = planComponents.find(c => c.id === id)
        return component ? [{ kind: 'component', line, component }] : []
      }
      case 'x:': {
        const index = pricing.components.extra.findIndex(
          (e, i) => e.name === id && !usedExtras.has(i)
        )
        if (index === -1) return []
        usedExtras.add(index)
        return [{ kind: 'extra', line, extra: pricing.components.extra[index], index }]
      }
      default: {
        const selection = addOns.find(a => a.addOnId === id)
        if (!selection) return []
        return [
          {
            kind: 'addOn',
            line,
            addOnId: id,
            quantity: selection.quantity ?? 1,
            addOn: availableAddOns.find(a => a.id === id),
          },
        ]
      }
    }
  })
  const excluded = planComponents.filter(c => pricing.components.removed.includes(c.id))

  const updateLine = (key: string, patch: Partial<LineDetailState>) =>
    setV2(prev => {
      const current = prev.lineDetails[key] ?? { description: '', discountPercent: '' }
      return { ...prev, lineDetails: { ...prev.lineDetails, [key]: { ...current, ...patch } } }
    })

  const move = (index: number, offset: -1 | 1) => {
    const order = rows.map(r => r.line.key)
    const target = index + offset
    ;[order[index], order[target]] = [order[target], order[index]]
    setV2(prev => ({ ...prev, lineOrder: order }))
  }

  const figuresOf = (price: Price | undefined, initialSlotCount?: number) =>
    price
      ? feeFigures(
          priceToSubscriptionFee(price, { initialSlotCount, creditUnits }),
          currency,
          creditUnits
        )
      : undefined

  const priceCell = (figures: LineFigures | undefined, custom = false) => (
    <FirstLine className="justify-end gap-1.5 whitespace-nowrap">
      {custom && (
        <Badge variant="outline" size="sm" className="font-normal text-muted-foreground">
          Custom
        </Badge>
      )}
      {figures ? <span title={figures.detail || undefined}>{figures.unitPrice}</span> : NONE}
    </FirstLine>
  )

  const exampleCells = (key: UsageExampleKey, name: string) => {
    const amount = exampleAmountByKey.get(key.componentId ?? key.name ?? '')
    const quantity = getExampleQuantity(pricing, key)
    return {
      quantity: (
        <ExampleQuantity
          name={name}
          quantity={quantity}
          onChange={value => actions.setExampleQuantity(key, value)}
        />
      ),
      amount:
        quantity.trim() !== '' && amount ? (
          <span className="text-muted-foreground">
            ≈ {formatCurrencyNoRounding(amount, currency)}
          </span>
        ) : (
          NONE
        ),
    }
  }

  const fixedQuantity = (figures: LineFigures | undefined) =>
    figures?.quantity !== undefined ? <span>{formatUsage(figures.quantity)}</span> : NONE

  const componentCells = (component: PriceComponent): RowCells => {
    const override = pricing.components.overridden.find(o => o.componentId === component.id)
    const configuration = pricing.components.parameterized.find(p => p.componentId === component.id)
    const feeType = deriveFeeType(component)
    const configurable = requiresConfiguration(component, pricing)
    const needsConfiguration = configurable && !isComponentConfigured(component, pricing)
    const price = planComponentDisplayPrice(component, configuration, override, currency)
    const figures = figuresOf(price, configuration?.initialSlotCount)
    const name = override?.name || component.name

    let quantity: ReactNode = fixedQuantity(figures)
    let amount: ReactNode = figures?.amount ?? NONE

    if (feeType === 'slot' && !override) {
      quantity = (
        <SeatQuantity
          name={name}
          value={configuration?.initialSlotCount}
          invalid={needsConfiguration}
          onChange={count => actions.updateConfiguration(component.id, { initialSlotCount: count })}
        />
      )
      amount = planComponentSlotTotal(component, configuration, override, currency) ?? NONE
    } else if (feeType === 'capacity' && configurable) {
      quantity = (
        <div className="w-40">
          <ComponentConfigurationFields
            component={component}
            configuration={configuration}
            currency={currency}
            onUpdateConfiguration={config => actions.updateConfiguration(component.id, config)}
          />
        </div>
      )
    } else if (price?.pricing.case === 'capacityPricing') {
      quantity = (
        <span className="text-muted-foreground">
          {formatUsage(Number(price.pricing.value.included))} incl.
        </span>
      )
    } else if (isExampleUsage(price)) {
      const example = exampleCells({ componentId: component.id }, name)
      quantity = example.quantity
      amount = example.amount
    }

    return {
      badge: FEE_BADGES[feeType],
      cadence: getComponentBillingLabel(component, configuration),
      quantity,
      price: priceCell(figures, Boolean(override)),
      amount,
      needsConfiguration,
      menu: (
        <LineMenu label={name}>
          <DropdownMenuItem onSelect={() => actions.startOverride(component.id)}>
            {override ? 'Edit custom price' : 'Override price'}
          </DropdownMenuItem>
          {override && (
            <DropdownMenuItem onSelect={() => actions.removeOverride(component.id)}>
              Reset to plan price
            </DropdownMenuItem>
          )}
          <DropdownMenuSeparator />
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            onSelect={() => actions.toggleRemoval(component.id)}
          >
            Exclude
          </DropdownMenuItem>
        </LineMenu>
      ),
    }
  }

  const extraCells = (extra: ExtraComponent, index: number): RowCells => {
    const price = formDataToPrice(extra.feeType, extra.formData, currency)
    const figures = figuresOf(price)
    let quantity: ReactNode = fixedQuantity(figures)
    let amount: ReactNode = figures?.amount ?? NONE
    if (isExampleUsage(price)) {
      const example = exampleCells({ name: extra.name }, extra.name)
      quantity = example.quantity
      amount = example.amount
    }
    return {
      badge: 'Extra',
      cadence: getPriceBillingLabel(price),
      quantity,
      price: priceCell(figures),
      amount,
      needsConfiguration: false,
      menu: (
        <LineMenu label={extra.name}>
          <DropdownMenuItem onSelect={() => actions.editExtra(index)}>Edit</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            onSelect={() => actions.removeExtra(index)}
          >
            Remove
          </DropdownMenuItem>
        </LineMenu>
      ),
    }
  }

  const addOnCells = (row: Extract<Row, { kind: 'addOn' }>): RowCells => {
    const { addOn, quantity, addOnId, line } = row
    const figures = figuresOf(addOn?.price)
    const maxInstances = addOn?.maxInstancesPerSubscription ?? null
    const supportsMultiple = maxInstances === null || maxInstances > 1
    // A quantity above one reads as the add-on selector always showed it, not as a computed sum.
    const amount = !figures?.amount ? (
      NONE
    ) : quantity > 1 ? (
      <span className="text-muted-foreground">
        {quantity} × {figures.amount}
      </span>
    ) : (
      figures.amount
    )

    return {
      badge: 'Add-on',
      cadence: addOn?.price ? getPriceBillingLabel(addOn.price) : undefined,
      quantity:
        addOn && supportsMultiple ? (
          <div className="flex items-center">
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              disabled={quantity <= 1}
              onClick={() => actions.setAddOnQuantity(addOnId, quantity - 1)}
              aria-label={`Fewer ${line.name}`}
            >
              <Minus className="h-3 w-3" />
            </Button>
            <span className="w-6 text-center tabular-nums" aria-live="polite">
              {quantity}
            </span>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              disabled={maxInstances !== null && quantity >= maxInstances}
              onClick={() => actions.setAddOnQuantity(addOnId, quantity + 1)}
              aria-label={`More ${line.name}`}
            >
              <Plus className="h-3 w-3" />
            </Button>
          </div>
        ) : (
          <span className="tabular-nums">{quantity}</span>
        ),
      price: priceCell(figures),
      amount,
      needsConfiguration: false,
      menu: (
        <LineMenu label={line.name}>
          <DropdownMenuItem
            className="text-destructive focus:text-destructive"
            onSelect={() => actions.removeAddOn(addOnId)}
          >
            Remove
          </DropdownMenuItem>
        </LineMenu>
      ),
    }
  }

  const renderRow = (row: Row, index: number) => {
    const cells =
      row.kind === 'component'
        ? componentCells(row.component)
        : row.kind === 'extra'
          ? extraCells(row.extra, row.index)
          : addOnCells(row)
    const name =
      row.kind === 'component'
        ? pricing.components.overridden.find(o => o.componentId === row.component.id)?.name ||
          row.line.name
        : row.line.name
    const detail = v2.lineDetails[row.line.key]
    const description = detail?.description ?? ''
    const discount = detail?.discountPercent ?? ''
    const error = discountError(discount)
    const editing = editingDescription === row.line.key

    return (
      <TableRow
        key={`${row.line.key}#${index}`}
        className={cn('group', cells.needsConfiguration && 'bg-warning/5')}
      >
        {lineEditing && (
          <TableCell className="w-14 py-2 align-top">
            <FirstLine>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="h-6 w-6"
                disabled={index === 0}
                onClick={() => move(index, -1)}
                aria-label={`Move ${name} up`}
              >
                <ArrowUp className="h-3.5 w-3.5" />
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="h-6 w-6"
                disabled={index === rows.length - 1}
                onClick={() => move(index, 1)}
                aria-label={`Move ${name} down`}
              >
                <ArrowDown className="h-3.5 w-3.5" />
              </Button>
            </FirstLine>
          </TableCell>
        )}
        <TableCell className="py-2 align-top">
          <FirstLine className="flex-wrap gap-x-2">
            <span className="font-medium text-foreground">{name}</span>
            {cells.badge && (
              <Badge variant="outline" size="sm" className="font-normal text-muted-foreground">
                {cells.badge}
              </Badge>
            )}
            {cells.cadence && (
              <span className="text-xs text-muted-foreground">{cells.cadence}</span>
            )}
            {lineEditing && !description && !editing && (
              <button
                type="button"
                onClick={() => setEditingDescription(row.line.key)}
                className="text-xs text-muted-foreground opacity-0 hover:text-foreground focus-visible:opacity-100 group-hover:opacity-100"
              >
                Add description
              </button>
            )}
          </FirstLine>
          {editing ? (
            <DescriptionInput
              name={name}
              initial={description}
              onCommit={value => {
                updateLine(row.line.key, { description: value })
                setEditingDescription(undefined)
              }}
              onCancel={() => setEditingDescription(undefined)}
            />
          ) : (
            description &&
            (lineEditing ? (
              <button
                type="button"
                onClick={() => setEditingDescription(row.line.key)}
                className="block max-w-md whitespace-pre-line text-left text-xs text-muted-foreground hover:text-foreground"
                aria-label={`Edit the description of ${name}`}
              >
                {description}
              </button>
            ) : (
              <p className="max-w-md whitespace-pre-line text-xs text-muted-foreground">
                {description}
              </p>
            ))
          )}
        </TableCell>
        <TableCell className="py-2 align-top">
          <FirstLine>{cells.quantity}</FirstLine>
        </TableCell>
        <TableCell className="py-2 text-right align-top">{cells.price}</TableCell>
        {lineEditing && (
          <TableCell className="py-2 align-top">
            <div className="relative w-20">
              <Input
                inputMode="decimal"
                value={discount}
                onChange={e => updateLine(row.line.key, { discountPercent: e.target.value })}
                placeholder="0"
                aria-label={`${name} discount, percent`}
                aria-invalid={Boolean(error)}
                title={error}
                className={cn('h-7 pr-6 text-right text-sm', error && 'border-destructive')}
              />
              <span className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-xs text-muted-foreground">
                %
              </span>
            </div>
          </TableCell>
        )}
        <TableCell className="py-2 text-right align-top">
          <FirstLine className="justify-end whitespace-nowrap tabular-nums">
            {cells.amount}
          </FirstLine>
        </TableCell>
        <TableCell className="w-10 py-2 align-top">
          <FirstLine>{cells.menu}</FirstLine>
        </TableCell>
      </TableRow>
    )
  }

  const columnCount = lineEditing ? 7 : 5

  return (
    <Table className="min-w-[760px]">
      <TableHeader>
        <TableRow>
          {lineEditing && (
            <TableHead className="w-14">
              <span className="sr-only">Order</span>
            </TableHead>
          )}
          <TableHead>Line</TableHead>
          <TableHead className="w-44">Quantity</TableHead>
          <TableHead className="w-40 text-right">Price</TableHead>
          {lineEditing && <TableHead className="w-24">Discount</TableHead>}
          <TableHead className="w-32 text-right">Amount</TableHead>
          <TableHead className="w-10">
            <span className="sr-only">Options</span>
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map(renderRow)}
        {rows.length === 0 && (
          <TableRow>
            <TableCell colSpan={columnCount} className="py-6 text-center text-muted-foreground">
              No lines
            </TableCell>
          </TableRow>
        )}
        {excluded.map(component => (
          <TableRow key={`excluded-${component.id}`} className="text-muted-foreground">
            {lineEditing && <TableCell />}
            <TableCell className="py-2">
              <span className="line-through">{component.name}</span>
            </TableCell>
            <TableCell colSpan={columnCount - (lineEditing ? 4 : 3)} />
            <TableCell colSpan={2} className="py-2 text-right">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => actions.toggleRemoval(component.id)}
                aria-label={`Include ${component.name}`}
              >
                Include
              </Button>
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}

const LineMenu = ({ label, children }: { label: string; children: ReactNode }) => (
  <DropdownMenu modal={false}>
    <DropdownMenuTrigger asChild>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-7 w-7"
        aria-label={`Options for ${label}`}
      >
        <MoreHorizontal className="h-4 w-4" />
      </Button>
    </DropdownMenuTrigger>
    <DropdownMenuContent align="end">{children}</DropdownMenuContent>
  </DropdownMenu>
)

const DescriptionInput = ({
  name,
  initial,
  onCommit,
  onCancel,
}: {
  name: string
  initial: string
  onCommit: (value: string) => void
  onCancel: () => void
}) => {
  const [value, setValue] = useState(initial)
  // Unmounting a focused input can fire a blur after Enter or Escape already settled it.
  const settled = useRef(false)
  const settle = (commit: boolean) => {
    if (settled.current) return
    settled.current = true
    if (commit) onCommit(value)
    else onCancel()
  }
  return (
    <Input
      autoFocus
      value={value}
      onChange={e => setValue(e.target.value)}
      onBlur={() => settle(true)}
      onKeyDown={e => {
        if (e.key === 'Enter') {
          e.preventDefault()
          settle(true)
        } else if (e.key === 'Escape') {
          e.preventDefault()
          settle(false)
        }
      }}
      placeholder="Description"
      aria-label={`${name} description`}
      className="mt-1 h-7 max-w-md text-xs"
    />
  )
}

const SeatQuantity = ({
  name,
  value,
  invalid,
  onChange,
}: {
  name: string
  value?: number
  invalid: boolean
  onChange: (count: number | undefined) => void
}) => (
  <div className="flex items-center gap-1.5">
    <Input
      type="number"
      min="0"
      value={value ?? ''}
      onChange={e => onChange(e.target.value ? parseInt(e.target.value) : undefined)}
      aria-label={`${name} seats`}
      aria-invalid={invalid}
      className={cn('h-7 w-16 px-2 text-sm', invalid && 'border-warning')}
    />
    <span className="text-xs text-muted-foreground">{value === 1 ? 'seat' : 'seats'}</span>
  </div>
)

const ExampleQuantity = ({
  name,
  quantity,
  onChange,
}: {
  name: string
  quantity: string
  onChange: (value: string) => void
}) => (
  <div className="group/example flex items-center gap-1.5">
    <span className="text-xs text-muted-foreground">est.</span>
    <Input
      type="number"
      min="0"
      value={quantity}
      onChange={e => onChange(e.target.value)}
      placeholder="—"
      aria-label={`${name}: example quantity shown on the quote`}
      className="h-7 w-24 px-2 text-sm"
    />
    {quantity !== '' && (
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="h-6 w-6 opacity-0 focus-visible:opacity-100 group-focus-within/example:opacity-100 group-hover/example:opacity-100"
        onClick={() => onChange('')}
        aria-label={`Clear the example quantity of ${name}`}
      >
        <X className="h-3 w-3" />
      </Button>
    )}
  </div>
)
