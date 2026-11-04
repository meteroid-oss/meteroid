import { useEffect, useRef, useState } from 'react'

import type { RefObject } from 'react'

/**
 * Tracks whether a horizontally-scrollable element is clipping content to its
 * left/right, so a fade can mark only the edges that actually hide something
 * instead of a fixed fade at both ends. Also scrolls the active child (by
 * `activeSelector`) into view once on mount, for a tab bar that loads with a
 * selection that is off-screen.
 */
export const useScrollFade = <T extends HTMLElement>(
  activeSelector = '[data-state="active"]'
): { ref: RefObject<T>; fadeLeft: boolean; fadeRight: boolean } => {
  const ref = useRef<T>(null)
  const [fadeLeft, setFadeLeft] = useState(false)
  const [fadeRight, setFadeRight] = useState(false)

  useEffect(() => {
    const el = ref.current
    if (!el) return

    const update = () => {
      setFadeLeft(el.scrollLeft > 1)
      setFadeRight(Math.ceil(el.scrollLeft + el.clientWidth) < el.scrollWidth - 1)
    }
    update()

    el.querySelector<HTMLElement>(activeSelector)?.scrollIntoView({
      inline: 'nearest',
      block: 'nearest',
    })

    el.addEventListener('scroll', update, { passive: true })
    const observer = new ResizeObserver(update)
    observer.observe(el)
    return () => {
      el.removeEventListener('scroll', update)
      observer.disconnect()
    }
    // Runs once: re-measures on scroll/resize instead of on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  return { ref, fadeLeft, fadeRight }
}
