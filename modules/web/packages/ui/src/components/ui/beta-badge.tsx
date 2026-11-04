import { Badge } from './badge'

export function BetaBadge() {
  return (
    <Badge variant="secondary" size="sm" className="italic">
      Beta
    </Badge>
  )
}

export function NewBadge() {
  return (
    <Badge variant="secondary" size="sm" className="italic">
      New
    </Badge>
  )
}

/** Marks a surface that belongs to a paid add-on the org doesn't own yet. */
export function AddonBadge({ label = 'Add-on' }: { label?: string }) {
  return (
    <Badge
      variant="outline"
      size="sm"
      className="border-brand/30 bg-brand/10 text-brand font-medium"
    >
      {label}
    </Badge>
  )
}
