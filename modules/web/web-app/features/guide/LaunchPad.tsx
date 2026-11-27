import { Button, Popover, PopoverContent, PopoverTrigger, cn } from '@md/ui'
import { useEffect, useState } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { toast } from 'sonner'

import CloseSvg from '@/assets/guide/close.svg?react'
import LauncherRingSvg from '@/assets/guide/launcher-ring.svg?react'
import MapSvg from '@/assets/guide/map.svg?react'
import StepDoneSvg from '@/assets/guide/step-done.svg?react'
import StepOpenSvg from '@/assets/guide/step-open.svg?react'
import { CustomersCreatePanel } from '@/features/customers'
import { useBasePath } from '@/hooks/useBasePath'
import { usePermissions } from '@/hooks/usePermissions'

import { GUIDE_TRACKS, GuideStep, GuideTrack } from './guideSteps'
import { useGuidePrefs } from './useGuidePrefs'
import { useGuideSignals } from './useGuideProgress'

/**
 * The Launch pad: a map button in the bottom-right corner that opens a short, hands-on tour of
 * Meteroid. Each track (Billing, CPQ, Metering) is a few concrete steps; a step checks itself off
 * once the tenant's data shows it was done, and takes you straight to the screen that does it.
 * It opens by itself on the first visit to Home, and can be put away (the organisation menu
 * brings it back).
 */
export const LaunchPad = () => {
  const prefs = useGuidePrefs()
  const visible = prefs.ready && !prefs.hidden
  const signals = useGuideSignals(visible)
  const { has, isLoading: permissionsLoading } = usePermissions()
  const navigate = useNavigate()
  const { pathname } = useLocation()
  const basePath = useBasePath()

  const [open, setOpen] = useState(false)
  const [trackId, setTrackId] = useState<GuideTrack['id']>('billing')
  const [expanded, setExpanded] = useState<string | null>(null)
  const [createCustomer, setCreateCustomer] = useState(false)

  const isDone = (step: GuideStep) =>
    step.signal ? signals[step.signal] : prefs.visited.includes(step.id)
  const allSteps = GUIDE_TRACKS.flatMap(track => track.steps)
  const remaining = allSteps.filter(step => !isDone(step)).length
  const track = GUIDE_TRACKS.find(t => t.id === trackId) ?? GUIDE_TRACKS[0]

  // Opening lands on the first track with something left to do, with its next step unfolded.
  const handleOpenChange = (next: boolean) => {
    setOpen(next)
    if (!next) return
    const firstOpen = GUIDE_TRACKS.find(t => t.steps.some(step => !isDone(step))) ?? GUIDE_TRACKS[0]
    setTrackId(firstOpen.id)
    setExpanded(firstOpen.steps.find(step => !isDone(step))?.id ?? null)
  }

  // Welcome new users: open once, the first time they land on Home.
  const onHome = pathname === basePath || pathname === `${basePath}/`
  useEffect(() => {
    if (visible && onHome && !prefs.autoOpened) {
      prefs.markAutoOpened()
      handleOpenChange(true)
    }
    // Runs when arriving on Home; the handler reads the latest progress itself.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visible, onHome, prefs.autoOpened])

  if (!visible) return null

  const takeStep = (step: GuideStep) => {
    setOpen(false)
    if (!step.signal) prefs.markVisited(step.id)
    if (step.cta.action === 'createCustomer') {
      setCreateCustomer(true)
      return
    }
    navigate(step.cta.to ? `${basePath}/${step.cta.to}` : basePath)
  }

  const hide = () => {
    setOpen(false)
    prefs.setHidden(true)
    toast('Launch pad put away', {
      description: 'Bring it back any time from the organisation menu.',
    })
  }

  return (
    <>
      <Popover open={open} onOpenChange={handleOpenChange}>
        <PopoverTrigger asChild>
          <button
            type="button"
            aria-label={
              remaining > 0 ? `Launch pad, ${remaining} steps to go` : 'Launch pad, all steps done'
            }
            title="Launch pad"
            className={cn(
              // Above a list page's footer (pagination sits in that corner).
              'fixed right-5 bottom-5 z-40 flex size-12 items-center justify-center rounded-full text-sidebar-foreground transition-[color,scale] duration-150 hover:text-foreground motion-safe:active:scale-95 [body:has([data-full-bleed])_&]:bottom-16',
              open && 'text-foreground'
            )}
          >
            <LauncherRingSvg aria-hidden className="pointer-events-none absolute -inset-[3px]" />
            <MapSvg aria-hidden className="relative [&_[stroke]]:stroke-current" />
            {remaining > 0 && (
              <span
                aria-hidden
                className="absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-checkbox-checked px-1 text-[10px] leading-none font-medium text-white tabular-nums"
              >
                {remaining}
              </span>
            )}
          </button>
        </PopoverTrigger>
        <PopoverContent
          side="top"
          align="end"
          sideOffset={12}
          className="flex w-[479px] max-w-[calc(100vw-2.5rem)] flex-col rounded-xl bg-card p-0 text-foreground dark:bg-[#222]"
        >
          <div className="flex justify-end px-5 pt-4 pb-2">
            <button
              type="button"
              onClick={() => setOpen(false)}
              aria-label="Close"
              className="flex size-7 items-center justify-center rounded-full text-faint-foreground transition-colors hover:bg-foreground/10 hover:text-foreground"
            >
              <CloseSvg aria-hidden className="rotate-45 [&_path]:fill-current" />
            </button>
          </div>

          <div className="flex items-baseline justify-between gap-4 px-5 py-3">
            <h2 className="font-display text-xl leading-none font-medium">
              Try Meteroid out for yourself
            </h2>
            <span className="shrink-0 text-[13px] leading-none text-foreground/50 tabular-nums">
              {allSteps.length - remaining} of {allSteps.length} done
            </span>
          </div>

          <div role="tablist" aria-label="Tracks" className="flex gap-2 px-6 py-2">
            {GUIDE_TRACKS.map(t => {
              const complete = t.steps.every(isDone)
              return (
                <button
                  key={t.id}
                  type="button"
                  role="tab"
                  aria-selected={t.id === track.id}
                  onClick={() => {
                    setTrackId(t.id)
                    setExpanded(t.steps.find(step => !isDone(step))?.id ?? null)
                  }}
                  className={cn(
                    'flex h-8 items-center gap-1.5 rounded-md px-3 text-[13px] leading-5 font-medium tracking-[-0.13px] transition-colors',
                    t.id === track.id
                      ? 'bg-foreground/24 text-foreground'
                      : 'bg-foreground/8 text-foreground/80 hover:bg-foreground/12'
                  )}
                >
                  {t.label}
                  {complete && <span className="text-tag-success">✓</span>}
                </button>
              )
            })}
          </div>

          <ol className="flex flex-col px-5 pt-2 pb-4">
            {track.steps.map(step => {
              const done = isDone(step)
              const isOpen = expanded === step.id
              const allowed = !step.permission || permissionsLoading || has(step.permission)
              return (
                <li key={step.id} className="border-t">
                  <button
                    type="button"
                    aria-expanded={isOpen}
                    onClick={() => setExpanded(isOpen ? null : step.id)}
                    className="group flex w-full items-center justify-between gap-4 py-3 text-left"
                  >
                    <span
                      className={cn(
                        'text-[15px] leading-5 tracking-[-0.15px] transition-colors group-hover:text-foreground',
                        done ? 'text-foreground/50' : 'text-foreground/80'
                      )}
                    >
                      {step.title}
                    </span>
                    {done ? (
                      <StepDoneSvg aria-label="Done" className="shrink-0" />
                    ) : (
                      <StepOpenSvg
                        aria-hidden
                        className={cn(
                          'shrink-0 transition-transform duration-200',
                          isOpen && 'rotate-90'
                        )}
                      />
                    )}
                  </button>
                  {/* Unfolds smoothly: the row grows from 0fr to 1fr. */}
                  <div
                    className={cn(
                      'grid transition-[grid-template-rows,opacity] duration-200 ease-out',
                      isOpen ? 'grid-rows-[1fr] opacity-100' : 'grid-rows-[0fr] opacity-0'
                    )}
                  >
                    <div className="overflow-hidden">
                      <div className="flex flex-col items-start gap-3 pb-4">
                        <p className="text-[13px] leading-5 text-foreground/60">
                          {step.description}
                        </p>
                        {allowed ? (
                          <Button
                            size="sm"
                            variant={done ? 'secondary' : 'primary'}
                            tabIndex={isOpen ? undefined : -1}
                            onClick={() => takeStep(step)}
                          >
                            {step.cta.label}
                          </Button>
                        ) : (
                          <p className="text-xs text-foreground/40">
                            Ask an admin of your workspace to take this step.
                          </p>
                        )}
                      </div>
                    </div>
                  </div>
                </li>
              )
            })}
          </ol>

          <div className="flex items-center justify-between border-t px-5 py-3">
            <span className="text-xs text-foreground/40">
              {remaining === 0
                ? 'All set. You have tried everything here.'
                : 'Steps check themselves off as you go.'}
            </span>
            <button
              type="button"
              onClick={hide}
              className="text-xs text-foreground/50 transition-colors hover:text-foreground"
            >
              Put away
            </button>
          </div>
        </PopoverContent>
      </Popover>
      <CustomersCreatePanel visible={createCustomer} closePanel={() => setCreateCustomer(false)} />
    </>
  )
}
