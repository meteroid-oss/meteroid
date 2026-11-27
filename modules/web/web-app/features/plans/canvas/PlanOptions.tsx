import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Label,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from '@md/ui'
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  EyeIcon,
  EyeOffIcon,
  MoreHorizontalIcon,
  PencilIcon,
  PlusIcon,
  Trash2Icon,
} from 'lucide-react'
import { FormEvent, useState, type ReactNode } from 'react'

import { useConfirmationModal } from 'providers/ConfirmationProvider'

import type { OptionMutations } from './useOptionMutations'
import type { OptionLike } from '@/features/plans/currencies/priceCells'

// A plan's options: what a buyer picks when buying it ("Monthly", "Annual"). The table shows
// one at a time, picked like the currency, or all side by side; the API names each by its alias.

const ALIAS = /^[a-z0-9][a-z0-9_-]{0,39}$/

const HIDDEN_HINT =
  'Hidden: not on the pricing page or in the portal; usable in quotes, links and the API'

/** A draft-only control on a published version: shown disabled, so the row keeps its shape. */
export const EditedInDraft = ({ children }: { children: ReactNode }) => (
  <Tooltip>
    <TooltipTrigger asChild>
      <span tabIndex={0} className="inline-flex rounded-md focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
        {children}
      </span>
    </TooltipTrigger>
    <TooltipContent>Prices are edited in a draft</TooltipContent>
  </Tooltip>
)

/** The "+" after a switch: dashed, square. */
export const addButton = 'size-8 border-dashed text-muted-foreground hover:text-foreground'

/** "Add an option": its name; going from one option to two, the existing one's too. */
export const AddOption = ({
  options,
  mutations,
  onAdded,
}: {
  options: OptionLike[]
  mutations: OptionMutations
  onAdded?: (option: OptionLike) => void
}) => {
  const [open, setOpen] = useState(false)
  const [name, setName] = useState('')
  const only = options.length === 1 ? options[0] : undefined
  const [existing, setExisting] = useState(only?.name ?? '')

  const reset = (next: boolean) => {
    setOpen(next)
    if (next) {
      setName('')
      setExisting(only?.name ?? '')
    }
  }
  const valid = name.trim().length > 0 && (!only || existing.trim().length > 0)
  const submit = async (event: FormEvent) => {
    event.preventDefault()
    if (!valid) return
    try {
      if (only && existing.trim() !== only.name) await mutations.rename(only.id, existing.trim())
      const added = (await mutations.add(name.trim())).option
      setOpen(false)
      if (added) onAdded?.(added)
    } catch {
      // Toasted by the mutation.
    }
  }

  return (
    <Popover open={open} onOpenChange={reset}>
      <PopoverTrigger asChild>
        <Button variant="outline" size="icon" className={addButton} aria-label="Add an option">
          <PlusIcon size={14} />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72">
        <p className="mb-2 text-sm font-medium">Add an option</p>
        <form onSubmit={submit} className="space-y-3">
          <p className="text-xs text-muted-foreground">
            What buyers pick between. It starts with the prices of{' '}
            {options[0]?.name ?? 'the first option'}.
          </p>
          {only && (
            <div className="space-y-1.5">
              <Label htmlFor="existing-option">This option</Label>
              <Input
                id="existing-option"
                value={existing}
                placeholder="e.g. Monthly"
                onChange={e => setExisting(e.target.value)}
              />
            </div>
          )}
          <div className="space-y-1.5">
            <Label htmlFor="new-option">{only ? 'The new one' : 'Name'}</Label>
            <Input
              id="new-option"
              autoFocus
              value={name}
              placeholder="e.g. Annual"
              onChange={e => setName(e.target.value)}
            />
          </div>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="secondary" size="sm" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" size="sm" disabled={!valid || mutations.pending}>
              Add option
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
  )
}

/** The switch of the options the table shows, then "+" and the selected one's menu. */
export const OptionBar = ({
  options,
  selected,
  isDraft,
  readOnly,
  mutations,
  warns,
  onSelect,
  onRenamed,
}: {
  options: OptionLike[]
  selected: OptionLike
  isDraft: boolean
  /** The switch alone, without "+" and the menu. */
  readOnly?: boolean
  mutations?: OptionMutations
  /** The option lacks a price in the currency shown: a dot on its tab. */
  warns: (option: OptionLike) => boolean
  onSelect: (option: OptionLike) => void
  onRenamed?: (option: OptionLike) => void
}) => {
  const menu = (
    <Button
      variant="ghost"
      size="icon"
      className="size-8 text-muted-foreground"
      disabled={!isDraft}
      aria-label={`${selected.name} option actions`}
    >
      <MoreHorizontalIcon size={14} />
    </Button>
  )
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <div role="tablist" aria-label="Option" className="flex flex-wrap items-center gap-1.5">
        {options.map((option, index) => {
          const current = option.id === selected.id
          const tab = (
            <button
              key={option.id}
              type="button"
              role="tab"
              aria-selected={current}
              onClick={() => !current && onSelect(option)}
              className={cn(
                'inline-flex h-8 items-center gap-2 rounded-md border px-3 text-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
                current
                  ? 'border-brand bg-brand/10 text-foreground'
                  : 'border-border text-muted-foreground hover:text-foreground',
                option.hidden && 'text-muted-foreground'
              )}
            >
              {option.hidden && <EyeOffIcon size={12} aria-label="Hidden" />}
              {option.name}
              {index === 0 && options.length > 1 && (
                <span className="text-xs text-muted-foreground">default</span>
              )}
              {warns(option) && (
                <span
                  role="img"
                  aria-label={`${option.name} lacks a price`}
                  className="size-1.5 rounded-full bg-warning"
                />
              )}
            </button>
          )
          return option.hidden ? (
            <Tooltip key={option.id}>
              <TooltipTrigger asChild>{tab}</TooltipTrigger>
              <TooltipContent multiline className="max-w-64">{HIDDEN_HINT}</TooltipContent>
            </Tooltip>
          ) : (
            tab
          )
        })}
      </div>
      {readOnly ? null : isDraft && mutations ? (
        <>
          <AddOption options={options} mutations={mutations} onAdded={onSelect} />
          <OptionMenu option={selected} options={options} mutations={mutations} onRenamed={onRenamed}>
            {menu}
          </OptionMenu>
        </>
      ) : (
        <>
          <EditedInDraft>
            <Button variant="outline" size="icon" className={addButton} disabled aria-label="Add an option">
              <PlusIcon size={14} />
            </Button>
          </EditedInDraft>
          <EditedInDraft>{menu}</EditedInDraft>
        </>
      )}
    </div>
  )
}

/** An option's menu: its alias, then rename, hide or show, move, remove (not the last one).
 * `children` is its trigger. */
const OptionMenu = ({
  option,
  options,
  mutations,
  onRenamed,
  children,
}: {
  option: OptionLike
  options: OptionLike[]
  mutations: OptionMutations
  onRenamed?: (option: OptionLike) => void
  children: ReactNode
}) => {
  const confirm = useConfirmationModal()
  const [renaming, setRenaming] = useState(false)
  const index = options.findIndex(o => o.id === option.id)
  // The first option is the default, always shown: a hidden one never moves there.
  const canMove = (by: -1 | 1) => {
    const to = index + by
    if (to < 0 || to >= options.length) return false
    return !(to === 0 && option.hidden) && !(index === 0 && options[to].hidden)
  }
  const move = (by: -1 | 1) => {
    const ids = options.map(o => o.id)
    ids.splice(index, 1)
    ids.splice(index + by, 0, option.id)
    void mutations.reorder(ids).catch(() => undefined)
  }
  const remove = () =>
    confirm(() => void mutations.remove(option.id).catch(() => undefined), {
      header: `Remove ${option.name}`,
      message: 'Its prices go with it. Buyers pick between the other options.',
    })
  const toggleHidden = (
    <DropdownMenuItem
      disabled={index === 0}
      onSelect={() => void mutations.setHidden(option.id, !option.hidden).catch(() => undefined)}
    >
      {option.hidden ? (
        <EyeIcon size={12} className="mr-2" />
      ) : (
        <EyeOffIcon size={12} className="mr-2" />
      )}
      {option.hidden
        ? 'Show on the pricing page and portal'
        : 'Hide from the pricing page and portal'}
    </DropdownMenuItem>
  )

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
            <span className="font-mono">{option.alias}</span> · used by the API
          </DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => setRenaming(true)}>
            <PencilIcon size={12} className="mr-2" /> Rename
          </DropdownMenuItem>
          {index === 0 ? (
            // A disabled item takes no pointer: the wrapper carries the tooltip.
            <Tooltip>
              <TooltipTrigger asChild>
                <div>{toggleHidden}</div>
              </TooltipTrigger>
              <TooltipContent side="left">The default option is always shown</TooltipContent>
            </Tooltip>
          ) : (
            toggleHidden
          )}
          {canMove(-1) && (
            <DropdownMenuItem onSelect={() => move(-1)}>
              <ArrowLeftIcon size={12} className="mr-2" /> Move left
            </DropdownMenuItem>
          )}
          {canMove(1) && (
            <DropdownMenuItem onSelect={() => move(1)}>
              <ArrowRightIcon size={12} className="mr-2" /> Move right
            </DropdownMenuItem>
          )}
          {options.length > 1 && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={remove} className="text-destructive">
                <Trash2Icon size={12} className="mr-2" /> Remove
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      {renaming && (
        <RenameOption
          option={option}
          mutations={mutations}
          onClose={renamed => {
            setRenaming(false)
            if (renamed) onRenamed?.(renamed)
          }}
        />
      )}
    </>
  )
}

const RenameOption = ({
  option,
  mutations,
  onClose,
}: {
  option: OptionLike
  mutations: OptionMutations
  /** With the option as renamed, once saved. */
  onClose: (renamed?: OptionLike) => void
}) => {
  const [name, setName] = useState(option.name)
  const [alias, setAlias] = useState(option.alias)
  const aliasValid = ALIAS.test(alias)
  const valid = name.trim().length > 0 && aliasValid
  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (!valid) return
    mutations
      .rename(option.id, name.trim(), alias !== option.alias ? alias : undefined)
      .then(() => onClose({ ...option, name: name.trim(), alias }), () => undefined)
  }

  return (
    <Dialog open onOpenChange={open => !open && onClose()}>
      <DialogContent className="max-w-sm">
        <form onSubmit={submit} className="space-y-4">
          <DialogHeader>
            <DialogTitle>Rename {option.name}</DialogTitle>
            <DialogDescription>Buyers see its name on the pricing page and at checkout.</DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="option-name">Name</Label>
            <Input id="option-name" autoFocus value={name} onChange={e => setName(e.target.value)} />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="option-alias">Alias</Label>
            <Input
              id="option-alias"
              className="font-mono"
              value={alias}
              aria-invalid={!aliasValid}
              onChange={e => setAlias(e.target.value.trim().toLowerCase())}
            />
            <p className={aliasValid ? 'text-xs text-muted-foreground' : 'text-xs text-destructive'}>
              {aliasValid
                ? 'Used by the API. Kept by the next versions of the plan.'
                : 'Lowercase letters, digits, - and _, at most 40.'}
            </p>
          </div>
          <DialogFooter>
            <Button type="button" variant="secondary" size="sm" onClick={() => onClose()}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" size="sm" disabled={!valid || mutations.pending}>
              Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
