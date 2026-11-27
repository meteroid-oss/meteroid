/**
 * Overlay scrollbars, as macOS draws them: nothing at rest and no room taken from the layout,
 * a thin thumb over the content while it scrolls that fades out a second after it stops.
 *
 * Native scrollbars are hidden (styles/main.css): outside macOS, and on macOS too once styled,
 * Chrome and Firefox reserve a gutter for them, and their thumb cannot fade. Instead every
 * scroll is caught here, and a thumb is drawn over the scrolling element's edge, fixed to the
 * viewport. It can be dragged, and wheeling over it scrolls the element underneath.
 *
 * Skipped: elements that still show a native scrollbar (the customer portal styles its own),
 * Radix ScrollArea viewports (they draw their own), and `data-scrollbar="none"` opt-outs.
 */
const HIDE_AFTER_MS = 1000
const FADE_MS = 300
const MIN_THUMB_PX = 28
/** The thumb's hit area across; the visible bar inside it is thinner (see .mt-scrollbar). */
const HIT_PX = 12
const INSET_PX = 2

type Axis = 'y' | 'x'

type Bars = {
  thumbs: Partial<Record<Axis, HTMLElement>>
  timer?: number
  dragging: boolean
  hovered: boolean
}

const bars = new Map<Element, Bars>()

const isScrollable = (element: Element, axis: Axis) => {
  const style = getComputedStyle(element)
  const size = axis === 'y' ? element.clientHeight : element.clientWidth
  const content = axis === 'y' ? element.scrollHeight : element.scrollWidth
  if (content <= size + 1) return false
  const overflow = axis === 'y' ? style.overflowY : style.overflowX
  // The root scrolls the viewport unless its overflow is clipped; other elements only when set to.
  if (element === document.documentElement) return overflow !== 'hidden' && overflow !== 'clip'
  return overflow === 'auto' || overflow === 'scroll'
}

const isSkipped = (element: Element) =>
  element.matches('[data-scrollbar="none"], [data-radix-scroll-area-viewport]') ||
  getComputedStyle(element).scrollbarWidth !== 'none'

/** The element's scrolling box on screen, clipped to the viewport. */
const visibleBox = (element: Element) => {
  const root = element === document.documentElement
  const rect = root ? { left: 0, top: 0 } : element.getBoundingClientRect()
  const left = Math.max(0, rect.left + element.clientLeft)
  const top = Math.max(0, rect.top + element.clientTop)
  const right = Math.min(window.innerWidth, rect.left + element.clientLeft + element.clientWidth)
  const bottom = Math.min(window.innerHeight, rect.top + element.clientTop + element.clientHeight)
  return { left, top, width: Math.max(0, right - left), height: Math.max(0, bottom - top) }
}

/** Track and thumb lengths along an axis, and how far the scroll moves per pixel of thumb. */
const measure = (element: Element, axis: Axis, crossBar: boolean) => {
  const box = visibleBox(element)
  const along = axis === 'y' ? box.height : box.width
  const track = along - 2 * INSET_PX - (crossBar ? HIT_PX : 0)
  const size = axis === 'y' ? element.clientHeight : element.clientWidth
  const content = axis === 'y' ? element.scrollHeight : element.scrollWidth
  const thumb = Math.min(track, Math.max(MIN_THUMB_PX, (track * size) / content))
  const maxScroll = content - size
  const scrollPerPx = track > thumb ? maxScroll / (track - thumb) : 0
  return { box, track, thumb, maxScroll, scrollPerPx }
}

const place = (element: Element, state: Bars) => {
  const both = Boolean(state.thumbs.x && state.thumbs.y)
  for (const axis of ['y', 'x'] as const) {
    const thumb = state.thumbs[axis]
    if (!thumb) continue
    const { box, thumb: length, maxScroll, track } = measure(element, axis, both)
    const scrolled = axis === 'y' ? element.scrollTop : Math.abs(element.scrollLeft)
    const offset = maxScroll > 0 ? (scrolled / maxScroll) * (track - length) : 0
    const style = thumb.style
    if (axis === 'y') {
      style.left = `${box.left + box.width - HIT_PX}px`
      style.top = `${box.top + INSET_PX + offset}px`
      style.width = `${HIT_PX}px`
      style.height = `${length}px`
    } else {
      style.left = `${box.left + INSET_PX + offset}px`
      style.top = `${box.top + box.height - HIT_PX}px`
      style.width = `${length}px`
      style.height = `${HIT_PX}px`
    }
  }
}

const scheduleHide = (element: Element, state: Bars) => {
  window.clearTimeout(state.timer)
  if (state.dragging || state.hovered) return
  state.timer = window.setTimeout(() => {
    for (const thumb of Object.values(state.thumbs)) thumb?.removeAttribute('data-visible')
    state.timer = window.setTimeout(() => {
      for (const thumb of Object.values(state.thumbs)) thumb?.remove()
      bars.delete(element)
    }, FADE_MS)
  }, HIDE_AFTER_MS)
}

const createThumb = (element: Element, axis: Axis, state: Bars) => {
  const thumb = document.createElement('div')
  thumb.className = 'mt-scrollbar'
  thumb.dataset.axis = axis
  thumb.setAttribute('aria-hidden', 'true')

  thumb.addEventListener('pointerenter', () => {
    state.hovered = true
    scheduleHide(element, state)
  })
  thumb.addEventListener('pointerleave', () => {
    state.hovered = false
    scheduleHide(element, state)
  })

  thumb.addEventListener('pointerdown', event => {
    if (event.button !== 0) return
    // Keep the press to the thumb: outside it, a dialog would read it as a click away and close.
    event.preventDefault()
    event.stopPropagation()
    thumb.setPointerCapture(event.pointerId)
    state.dragging = true
    thumb.dataset.dragging = ''
    const start = axis === 'y' ? event.clientY : event.clientX
    const startScroll = axis === 'y' ? element.scrollTop : element.scrollLeft
    const { scrollPerPx } = measure(element, axis, Boolean(state.thumbs.x && state.thumbs.y))

    const move = (moveEvent: PointerEvent) => {
      const delta = (axis === 'y' ? moveEvent.clientY : moveEvent.clientX) - start
      if (axis === 'y') element.scrollTop = startScroll + delta * scrollPerPx
      else element.scrollLeft = startScroll + delta * scrollPerPx
    }
    const end = () => {
      thumb.removeEventListener('pointermove', move)
      thumb.removeEventListener('pointerup', end)
      thumb.removeEventListener('pointercancel', end)
      state.dragging = false
      delete thumb.dataset.dragging
      scheduleHide(element, state)
    }
    thumb.addEventListener('pointermove', move)
    thumb.addEventListener('pointerup', end)
    thumb.addEventListener('pointercancel', end)
  })

  // The thumb sits above the element, so a wheel over it would scroll the page instead.
  thumb.addEventListener(
    'wheel',
    event => {
      event.preventDefault()
      const unit = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : 1
      element.scrollBy({ left: event.deltaX * unit, top: event.deltaY * unit })
    },
    { passive: false }
  )

  return thumb
}

const show = (element: Element) => {
  let state = bars.get(element)
  if (!state) {
    if (isSkipped(element)) return
    state = { thumbs: {}, dragging: false, hovered: false }
  }

  const appended: HTMLElement[] = []
  for (const axis of ['y', 'x'] as const) {
    const scrollable = isScrollable(element, axis)
    const thumb = state.thumbs[axis]
    if (scrollable && !thumb) {
      const created = createThumb(element, axis, state)
      state.thumbs[axis] = created
      appended.push(created)
    } else if (!scrollable && thumb) {
      thumb.remove()
      delete state.thumbs[axis]
    }
  }
  if (!state.thumbs.x && !state.thumbs.y) {
    bars.delete(element)
    return
  }
  bars.set(element, state)

  place(element, state)
  if (appended.length > 0) {
    document.body.append(...appended)
    // Lay the new thumbs out at rest first, so they fade in rather than pop up.
    void appended[0].offsetWidth
  }
  for (const thumb of Object.values(state.thumbs)) thumb?.setAttribute('data-visible', '')
  scheduleHide(element, state)
}

export const showScrollbarsWhileScrolling = () => {
  document.addEventListener(
    'scroll',
    event => {
      const element =
        event.target === document ? document.documentElement : (event.target as Element | null)
      if (element instanceof Element) show(element)
    },
    // Scroll events do not bubble: listen in the capture phase to see every element's.
    { capture: true, passive: true }
  )
}
