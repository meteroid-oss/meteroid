import { cn } from '@md/ui'
import { Check } from 'lucide-react'

import { TEMPLATE_LOCALES } from '@/features/customer-emails/locales'
import { localeLabel } from '@/features/settings/utils/localeRecord'

// Supported order, without the default: keeps a form's dirty check stable.
export const sortLocales = (locales: string[], defaultLocale?: string): string[] =>
  TEMPLATE_LOCALES.filter(l => l !== defaultLocale && locales.includes(l))

/** Toggle pills picking a subset of `options`; the value stays in supported order. */
export const LocaleToggles = ({
  options,
  value,
  onChange,
  disabled,
}: {
  options: string[]
  value: string[]
  onChange: (value: string[]) => void
  disabled?: boolean
}) => (
  <div className="flex flex-wrap gap-1.5">
    {options.map(locale => {
      const on = value.includes(locale)
      return (
        <button
          key={locale}
          type="button"
          disabled={disabled}
          aria-pressed={on}
          onClick={() =>
            onChange(on ? value.filter(l => l !== locale) : sortLocales([...value, locale]))
          }
          className={cn(
            'inline-flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-[13px] transition-colors disabled:cursor-not-allowed disabled:opacity-60',
            on
              ? 'border-primary bg-accent text-foreground'
              : 'border-border text-muted-foreground hover:text-foreground'
          )}
        >
          {on && <Check size={13} />}
          {localeLabel(locale)}
        </button>
      )
    })}
  </div>
)
