/**
 * ASCII empty-state scenes for Meteroid.
 *
 * Every scene is a pure function of time `t` (in ticks, ~14 per second at
 * speed 1). It fills a COLS x ROWS character grid plus a level per cell:
 *   0 = dim, 1 = mid, 2 = bright (bright gets the tint colour and glow).
 */

export const COLS = 44
export const ROWS = 22

export type Level = 0 | 1 | 2
export type Frame = { ch: string[]; lv: Level[] }
export type SceneName =
  | 'customers'
  | 'plans'
  | 'invoices'
  | 'events'
  | 'reports'
  | 'subscriptions'
  | 'products'
  | 'metrics'
  | 'features'
  | 'addons'
  | 'wallet'

const W = COLS
const H = ROWS
const mk = (): Frame => ({
  ch: new Array(W * H).fill(' '),
  lv: new Array(W * H).fill(0) as Level[],
})

/** Customers: a round badge with a person icon that spins like a coin. Only the front catches the shine. */
type CoinPoint = { x: number; y: number; z: number; u: number; side: number; m: number }
// Materials: 0 outer rim, 1 tick on, 2 tick off, 3 person icon, 5 background, 6 coin edge
let coinPoints: CoinPoint[] | null = null
const COIN_R = 17,
  COIN_TH = 2.6

/** The coin's shape never changes, so its points are sampled once and only rotated per frame. */
function getCoinPoints(): CoinPoint[] {
  if (coinPoints) return coinPoints
  const R = COIN_R,
    pts: CoinPoint[] = []
  for (const side of [-1, 1]) {
    const z = (side * COIN_TH) / 2
    for (let x = -R; x <= R; x += 0.32) {
      for (let y = -R; y <= R; y += 0.42) {
        const r = Math.hypot(x, y)
        if (r > R) continue
        const u = side < 0 ? x : -x
        let m: number
        if (r > R - 1.5) m = 0
        else if (r > R - 3.2 && r < R - 2.3) m = Math.sin(Math.atan2(y, x) * 18) > 0 ? 1 : 2
        else if (
          Math.hypot(u, y + 4) < 4 ||
          ((u / 7.6) ** 2 + ((y - 8.4) / 5.2) ** 2 < 1 && y < 8.4 && r < R - 3.4)
        )
          m = 3
        else m = 5
        pts.push({ x, y, z, u, side, m })
      }
    }
  }
  for (let a = 0; a < Math.PI * 2; a += 0.02) {
    for (let z = -COIN_TH / 2; z <= COIN_TH / 2; z += 0.45) {
      pts.push({ x: R * Math.cos(a), y: R * Math.sin(a), z, u: Math.cos(a), side: 0, m: 6 })
    }
  }
  coinPoints = pts
  return pts
}

function customers(t: number): Frame {
  const f = mk()
  const zb = new Array(W * H).fill(-1e9)
  const AR = 1.82,
    d = 110
  const base = t * 0.05,
    th = base - 0.5 * Math.sin(base)
  const c = Math.cos(th),
    s = Math.sin(th)
  const Ld = [-0.35, -0.45, -0.82],
    Ln = Math.hypot(Ld[0], Ld[1], Ld[2])
  const glint = Math.sin(th) * 34
  const lum = (nx: number, nz: number) => {
    const X = nx * c + nz * s,
      Z = -nx * s + nz * c
    return Math.max(0, (X * Ld[0] + Z * Ld[2]) / Ln)
  }
  const bFront = 0.2 + 0.8 * lum(0, -1),
    bBack = 0.2 + 0.8 * lum(0, 1)
  for (const p of getCoinPoints()) {
    const X = p.x * c + p.z * s,
      Z = -p.x * s + p.z * c,
      ooz = 1 / (d + Z)
    const col = Math.floor(W / 2 + X * d * ooz),
      row = Math.floor(10.2 + (p.y * d * ooz) / AR)
    if (col < 0 || col >= W || row < 0 || row >= H) continue
    const i = row * W + col
    if (ooz <= zb[i]) continue
    zb[i] = ooz
    const b = p.side < 0 ? bFront : p.side > 0 ? bBack : 0.2 + 0.8 * lum(p.u, 0)
    let k: string, l: Level
    switch (p.m) {
      // Rim and person lighter than the other scenes' solids ('+', '=' and '#' rather than '@',
      // '%'), and bright only in the strongest light: the whole face is lit at once, so a lower
      // threshold turns the full badge bright together and it reads heavy next to the others.
      case 0:
        k = b > 0.6 ? '+' : b > 0.35 ? '=' : ':'
        l = b > 0.75 ? 2 : 1
        break
      case 1:
        k = b > 0.4 ? '-' : '.'
        l = b > 0.5 ? 1 : 0
        break
      case 3:
        k = b > 0.7 ? '#' : b > 0.5 ? '+' : b > 0.3 ? '=' : ':'
        l = b > 0.92 ? 2 : 1
        break
      case 6:
        k = b > 0.5 ? '#' : b > 0.3 ? '=' : ':'
        l = b > 0.5 ? 1 : 0
        break
      case 5:
        if (p.side < 0 && b > 0.45 && Math.abs(p.u * 0.9 + p.y * 0.5 - glint) < 1.2) {
          k = '/'
          l = 1
          break
        }
        k = '.'
        l = 0
        break
      default:
        k = '.'
        l = 0
    }
    f.ch[i] = k
    f.lv[i] = l
  }
  const w = COIN_R * Math.max(Math.abs(c), 0.1) + 3
  for (let col = 0; col < W; col++) {
    const sx = col - W / 2 + 0.5,
      i = (H - 1) * W + col
    if (Math.abs(sx) < w) {
      f.ch[i] = Math.abs(sx) < w * 0.55 ? ':' : '.'
      f.lv[i] = 0
    }
  }
  return f
}

/** Plans: a shaded cube with bright edges, turning on a fixed tilt. */
function plans(t: number): Frame {
  const f = mk()
  const zb = new Array(W * H).fill(-1e9)
  const A = -0.55,
    B = t * 0.05 + 0.7,
    RA = '.:-=+*#'
  const cA = Math.cos(A),
    sA = Math.sin(A),
    cB = Math.cos(B),
    sB = Math.sin(B)
  const rot = (x: number, y: number, z: number): [number, number, number] => {
    const x1 = x * cB + z * sB,
      z1 = -x * sB + z * cB
    return [x1, y * cA - z1 * sA, y * sA + z1 * cA]
  }
  const L = [-0.5, 0.8, -0.5],
    ln = Math.hypot(L[0], L[1], L[2])
  const faces: [number, number, number][] = [
    [0, 0, 1],
    [0, 0, -1],
    [0, 1, 0],
    [0, -1, 0],
    [1, 0, 0],
    [-1, 0, 0],
  ]
  const K2 = 5,
    K1 = 27
  for (const n of faces) {
    const [nx, ny, nz] = rot(n[0], n[1], n[2])
    const lum = (nx * L[0] + ny * L[1] + nz * L[2]) / ln
    for (let a = -1; a <= 1.0001; a += 0.04) {
      for (let b = -1; b <= 1.0001; b += 0.04) {
        const p: [number, number, number] = n[0] ? [n[0], a, b] : n[1] ? [a, n[1], b] : [a, b, n[2]]
        const [x, y, z] = rot(p[0], p[1], p[2])
        const ooz = 1 / (z + K2)
        const xp = Math.floor(W / 2 + K1 * ooz * x),
          yp = Math.floor(H / 2 - K1 * ooz * y * 0.55)
        if (xp < 0 || xp >= W || yp < 0 || yp >= H) continue
        const i = yp * W + xp
        if (ooz > zb[i] + 1e-6) {
          zb[i] = ooz
          if (Math.abs(a) > 0.94 || Math.abs(b) > 0.94) {
            f.ch[i] = '#'
            f.lv[i] = 2
          } else if (lum > 0) {
            const k = Math.min(6, Math.round(lum * 6))
            f.ch[i] = RA[k]
            f.lv[i] = k >= 4 ? 1 : 0
          } else {
            f.ch[i] = '.'
            f.lv[i] = 0
          }
        }
      }
    }
  }
  return f
}

/** Invoices: a dot-matrix printer printing an invoice on tractor-feed paper. */
function invoices(t: number): Frame {
  const f = mk()
  const set = (r: number, c: number, k: string, l: Level) => {
    if (r >= 0 && r < H && c >= 0 && c < W) {
      f.ch[r * W + c] = k
      f.lv[r * W + c] = l
    }
  }
  const lines = [
    'INVOICE          #0042',
    'Nov 26, 2026',
    '',
    'Plan           € 49,00',
    'Usage          € 12,40',
    'Seats x3       € 27,00',
    '--------------------',
    'Total          € 88,40',
    '',
    '',
    '',
    '',
    '',
  ]
  const total = lines.reduce((sum, l) => sum + Math.max(l.length, 3), 0) + 40
  let tt = (t * 0.9) % total,
    n: number,
    partial = 0
  for (n = 0; n < lines.length; n++) {
    const len = Math.max(lines[n].length, 3)
    if (tt < len) {
      partial = Math.floor(tt)
      break
    }
    tt -= len
  }
  const done = n >= lines.length
  const X0 = 9,
    PW = 26,
    vis = 16
  const printed = lines.slice(0, Math.min(n, lines.length))
  if (!done) printed.push(lines[n].slice(0, partial))
  const start = Math.max(0, printed.length - vis),
    shown = printed.slice(start),
    off = vis - shown.length
  for (let r = off; r < vis; r++) {
    const li = r - off,
      txt = shown[li],
      cur = !done && li === shown.length - 1
    set(r, X0, '|', 0)
    set(r, X0 + PW - 1, '|', 0)
    const hole = (start + li) % 2 === 0 ? 'o' : ' '
    set(r, X0 + 1, hole, 0)
    set(r, X0 + PW - 2, hole, 0)
    for (let k = 0; k < txt.length; k++) if (txt[k] !== ' ') set(r, X0 + 3 + k, txt[k], cur ? 2 : 1)
    if (cur) set(r, X0 + 3 + txt.length, '█', 2)
  }
  if (start === 0) for (let c = X0; c < X0 + PW; c++) set(off - 1, c, c % 2 ? '-' : '~', 0)
  const inner = 38,
    row = (s: string) => '  |' + s.padEnd(inner, ' ') + '|'
  const body = [
    '  .' + '-'.repeat(5) + '[' + '='.repeat(PW) + ']' + '-'.repeat(5) + '.',
    row(''),
    row('   ' + ':'.repeat(32)),
    row('  [::]  LQ-42'),
    "  '" + '-'.repeat(inner) + "'",
  ]
  body.forEach((s, ri) => {
    for (let c = 0; c < s.length; c++) if (s[c] !== ' ') set(vis + ri, c, s[c], 0)
  })
  for (let c = X0; c < X0 + PW; c++) set(vis, c, '=', 1)
  const led = Math.floor(t / 7) % 2 === 0
  set(vis + 3, 34, led ? '●' : '○', led ? 2 : 0)
  set(vis + 3, 37, '○', 0)
  return f
}

/** Events: a radar sweep listening for incoming events. */
function events(t: number): Frame {
  const f = mk()
  const cx = W / 2 - 0.5,
    cy = H / 2 - 0.5,
    RX = 19.5,
    AR = 1.82,
    TAU = Math.PI * 2,
    phi = (t * 0.11) % TAU
  const set = (r: number, c: number, k: string, l: Level) => {
    if (r >= 0 && r < H && c >= 0 && c < W) {
      f.ch[r * W + c] = k
      f.lv[r * W + c] = l
    }
  }
  // The beam and its fading trail
  for (let r = 0; r < H; r++) {
    for (let c = 0; c < W; c++) {
      const dx = c - cx,
        dy = (r - cy) * AR,
        dist = Math.hypot(dx, dy) / RX
      if (dist >= 0.95) continue
      let del = (phi - Math.atan2(dy, dx)) % TAU
      if (del < 0) del += TAU
      if (del < 0.12) {
        set(r, c, '#', 2)
        continue
      }
      if (del < 1.6) {
        const idx = Math.round((1 - del / 1.6) * 6)
        if (idx > 0) set(r, c, ' .:-=+*'[idx], idx >= 4 ? 1 : 0)
      }
    }
  }
  // Rings, drawn along the circle; they light up just behind the beam
  const ring = (q: number, outer: boolean) => {
    for (let a = 0; a < TAU; a += 0.01) {
      const c = Math.round(cx + Math.cos(a) * RX * q),
        r = Math.round(cy + (Math.sin(a) * RX * q) / AR)
      let del = (phi - a) % TAU
      if (del < 0) del += TAU
      const lit = del < 1.2
      if (outer) set(r, c, '.', lit ? 2 : 1)
      else if (Math.round(a * 100) % 3 === 0) set(r, c, '·', lit ? 2 : 0)
    }
  }
  ring(0.36, false)
  ring(0.68, false)
  ring(1, true)
  set(Math.round(cy), Math.round(cx), 'o', 2)
  return f
}

/** Reports: bars rising and falling with eighth-block caps. */
function reports(t: number): Frame {
  const f = mk()
  const set = (r: number, c: number, k: string, l: Level) => {
    if (r >= 0 && r < H && c >= 0 && c < W) {
      f.ch[r * W + c] = k
      f.lv[r * W + c] = l
    }
  }
  const base = 19,
    maxH = 17,
    n = 9,
    bw = 3,
    gap = 1,
    x0 = 5,
    E = ' ▁▂▃▄▅▆▇█'
  for (const q of [0.25, 0.5, 0.75, 1]) {
    const r = base - Math.round(q * maxH)
    for (let c = x0 - 1; c < W - 2; c += 2) set(r, c, '·', 0)
  }
  for (let c = x0 - 1; c < W - 2; c++) set(base, c, '─', 1)
  set(base, x0 - 2, '└', 1)
  for (let r = 1; r < base; r++) set(r, x0 - 2, '│', 0)
  for (let k = 0; k < n; k++) {
    const h =
      (0.42 +
        0.22 * Math.sin(t * 0.07 + k * 0.75) +
        0.16 * Math.sin(t * 0.031 + k * 1.9) +
        k * 0.025) *
      maxH
    const full = Math.floor(h),
      fr = h - full,
      cx = x0 + k * (bw + gap)
    for (let j = 0; j < full; j++) for (let w = 0; w < bw; w++) set(base - 1 - j, cx + w, '█', 0)
    const e = E[Math.round(fr * 8)]
    for (let w = 0; w < bw; w++) {
      if (e !== ' ') set(base - 1 - full, cx + w, e, 2)
      else if (full > 0) set(base - full, cx + w, '█', 2)
    }
  }
  const lbl = 'JFMAMJJAS'
  for (let k = 0; k < n; k++) set(base + 2, x0 + k * (bw + gap) + 1, lbl[k], 0)
  return f
}

/** Subscriptions: a lit, spinning wireframe planet with a moon on an orbit. */
function subscriptions(t: number): Frame {
  const f = mk()
  const zb = new Array(W * H).fill(0)
  const cx = W / 2 - 0.5,
    cy = H / 2 - 0.5,
    AR = 1.82,
    R = 8.8,
    spin = t * 0.035
  const L = [-0.6, -0.5, 0.62],
    ln = Math.hypot(L[0], L[1], L[2]),
    RA = ' .:-=+*#%@'
  for (let r = 0; r < H; r++) {
    for (let c = 0; c < W; c++) {
      const x = c - cx,
        y = (r - cy) * AR,
        q = x * x + y * y
      if (q > R * R) continue
      const z = Math.sqrt(R * R - q),
        nx = x / R,
        ny = y / R,
        nz = z / R,
        i = r * W + c
      const lum = (nx * L[0] + ny * L[1] + nz * L[2]) / ln
      const lon = Math.atan2(nx, nz) + spin,
        lat = Math.asin(ny)
      const grid = Math.abs(Math.sin(lon * 3)) < 0.13 || Math.abs(Math.sin(lat * 3.5)) < 0.12
      zb[i] = 1
      if (lum <= 0.04) {
        if (q > (R - 1.3) * (R - 1.3)) {
          f.ch[i] = '.'
          f.lv[i] = 1
        } else {
          f.ch[i] = grid ? '.' : ' '
          f.lv[i] = 0
        }
        continue
      }
      if (grid) {
        f.ch[i] = lum > 0.5 ? '#' : lum > 0.2 ? '+' : ':'
        f.lv[i] = lum > 0.12 ? 2 : 1
        continue
      }
      const k = Math.round(Math.pow(lum, 0.8) * 9)
      f.ch[i] = RA[Math.max(1, k)]
      f.lv[i] = k >= 6 ? 2 : k >= 3 ? 1 : 0
    }
  }
  const RX = 20,
    RY = 7.2,
    tl = 0.11
  const pt = (a: number): [number, number] => {
    const x = RX * Math.cos(a)
    return [Math.round(cx + x), Math.round(cy + RY * Math.sin(a) - x * tl)]
  }
  for (let a = 0; a < Math.PI * 2; a += 0.015) {
    const [c, r] = pt(a)
    if (r < 0 || r >= H || c < 0 || c >= W) continue
    const i = r * W + c,
      back = Math.sin(a) < 0
    if (back && zb[i]) continue
    if (f.ch[i] === ' ' || !back) {
      f.ch[i] = back ? '.' : '·'
      f.lv[i] = back ? 0 : 1
    }
  }
  const m = (t * 0.045) % (Math.PI * 2)
  for (let k = 5; k >= 0; k--) {
    const a = m - k * 0.07,
      [c, r] = pt(a)
    if (r < 0 || r >= H || c < 0 || c >= W) continue
    const i = r * W + c
    if (Math.sin(a) < 0 && zb[i]) continue
    f.ch[i] = k === 0 ? '@' : k < 3 ? 'o' : '.'
    f.lv[i] = k < 3 ? 2 : 1
  }
  return f
}

/** Products: a lumpy, cratered meteoroid tumbling through space while the stars drift past. */
function products(t: number): Frame {
  const f = mk()
  const AR = 1.82
  // Stars drift diagonally past, so the rock seems to travel
  for (let k = 0; k < 16; k++) {
    const sx = (k * 97.3) % W,
      sy = (k * 53.9) % H
    const c = Math.floor((sx + t * 0.12 * (1 + (k % 3) * 0.5)) % W)
    const r = Math.floor((sy + t * 0.05 * (1 + (k % 3) * 0.5)) % H)
    const v = Math.sin(t * 0.17 + k * 2.3)
    const i = r * W + c
    f.ch[i] = v > 0.6 ? '+' : k % 3 === 2 ? '·' : '.'
    f.lv[i] = v > 0.6 ? 2 : 0
  }
  const cx = W / 2 + 1,
    cy = H / 2 - 0.5,
    R0 = 11
  // Tumble: spin around an axis tilted towards the viewer
  const spin = t * 0.04
  const ax = [0.35, 0.85, 0.4],
    an = Math.hypot(ax[0], ax[1], ax[2])
  const kx = ax[0] / an,
    ky = ax[1] / an,
    kz = ax[2] / an
  const cs = Math.cos(-spin),
    sn = Math.sin(-spin)
  // Rodrigues rotation: world-space normal -> rock (body) space
  const toBody = (x: number, y: number, z: number): [number, number, number] => {
    const dot = kx * x + ky * y + kz * z
    const cxp = ky * z - kz * y,
      cyp = kz * x - kx * z,
      czp = kx * y - ky * x
    return [
      x * cs + cxp * sn + kx * dot * (1 - cs),
      y * cs + cyp * sn + ky * dot * (1 - cs),
      z * cs + czp * sn + kz * dot * (1 - cs),
    ]
  }
  // Lumpy outline: radius per screen angle, shifting as the rock turns
  const radius = (a: number) =>
    R0 *
    (1 +
      0.09 * Math.sin(3 * a + spin * 1.3) +
      0.06 * Math.sin(5 * a - spin * 0.9 + 1.2) +
      0.04 * Math.sin(7 * a + spin * 2.1))
  const craters: [number, number, number, number][] = [
    [0.6, 0.3, 0.74, 0.42],
    [-0.5, 0.5, 0.7, 0.32],
    [0.1, -0.8, 0.6, 0.38],
    [-0.7, -0.4, 0.6, 0.28],
    [0.2, 0.9, -0.4, 0.3],
    [-0.3, 0.1, -0.95, 0.45],
    [0.9, -0.2, -0.4, 0.26],
    [-0.9, 0.3, -0.3, 0.3],
  ].map(([x, y, z, rad]) => {
    const n = Math.hypot(x, y, z)
    return [x / n, y / n, z / n, rad]
  })
  const L = [-0.55, -0.6, 0.58],
    Ln = Math.hypot(L[0], L[1], L[2])
  const RAMP = ' .:-=+*#%@'
  for (let r = 0; r < H; r++) {
    for (let c = 0; c < W; c++) {
      const dx = c - cx,
        dy = (r - cy) * AR
      const rr = radius(Math.atan2(dy, dx))
      const q = dx * dx + dy * dy
      if (q > rr * rr) continue
      const nx = dx / rr,
        ny = dy / rr,
        nz = Math.sqrt(Math.max(0, 1 - (nx * nx + ny * ny)))
      let lum = (nx * L[0] + ny * L[1] + nz * L[2]) / Ln
      const [bx, by, bz] = toBody(nx, ny, nz)
      let rim = false
      for (const [px, py, pz, rad] of craters) {
        const d = Math.acos(Math.max(-1, Math.min(1, bx * px + by * py + bz * pz)))
        if (d < rad) {
          const depth = 1 - d / rad
          lum = lum * (0.2 + 0.35 * (1 - depth)) - 0.1 * depth // shadowed bowl
          if (d > rad * 0.75) rim = true
        }
      }
      lum += 0.07 * Math.sin(bx * 9 + by * 4) * Math.cos(bz * 7 - bx * 3) // rocky texture
      const i = r * W + c
      const edge = q > (rr - 1.1) * (rr - 1.1)
      if (lum <= 0.03) {
        f.ch[i] = edge ? '.' : ' '
        f.lv[i] = edge ? 1 : 0
        continue
      }
      let k = Math.max(1, Math.min(9, Math.round(lum * 8.5)))
      if (rim && lum > 0.15) k = Math.min(9, k + 3)
      f.ch[i] = RAMP[k]
      f.lv[i] = k >= 7 ? 2 : k >= 4 ? 1 : 0
    }
  }
  // A few small fragments tumbling along
  for (let k = 0; k < 3; k++) {
    const a = t * 0.03 * (k % 2 ? 1 : -1) + k * 2.1
    const c = Math.round(cx + Math.cos(a) * (15 + k * 2)),
      r = Math.round(cy + (Math.sin(a) * (12 + k * 2)) / AR)
    if (r >= 0 && r < H && c >= 0 && c < W && f.ch[r * W + c] === ' ') {
      f.ch[r * W + c] = k === 0 ? '*' : 'o'
      f.lv[r * W + c] = 1
    }
  }
  return f
}

/** Metrics: an analog gauge whose needle swings with usage, above a rolling counter. */
function metrics(t: number): Frame {
  const f = mk()
  const set = (r: number, c: number, k: string, l: Level) => {
    if (r >= 0 && r < H && c >= 0 && c < W) {
      f.ch[r * W + c] = k
      f.lv[r * W + c] = l
    }
  }
  const cx = 21.5,
    cy = 15,
    RX = 18,
    AR = 1.82
  const at = (a: number, q: number): [number, number] => [
    Math.round(cy - (Math.sin(a) * RX * q) / AR),
    Math.round(cx + Math.cos(a) * RX * q),
  ]
  for (let a = 0; a <= Math.PI + 1e-6; a += 0.012) {
    const [r, c] = at(a, 1)
    set(r, c, '.', 1)
  }
  for (let i = 0; i <= 20; i++) {
    const a = Math.PI - (i / 20) * Math.PI,
      major = i % 5 === 0
    if (major)
      for (let q = 0.8; q <= 0.9; q += 0.05) {
        const [r, c] = at(a, q)
        set(r, c, '#', i >= 15 ? 2 : 1)
      }
    else {
      const [r, c] = at(a, 0.88)
      set(r, c, '·', i >= 16 ? 1 : 0)
    }
  }
  const v = 0.45 + 0.3 * Math.sin(t * 0.045) + 0.07 * Math.sin(t * 0.31)
  const a = Math.PI - v * Math.PI
  const sl = Math.cos(a) / (Math.sin(a) + 1e-6)
  const k = Math.abs(sl) > 2.2 ? '-' : Math.abs(sl) < 0.45 ? '|' : sl > 0 ? '/' : '\\'
  for (let q = 0.08; q <= 0.74; q += 0.02) {
    const [r, c] = at(a, q)
    set(r, c, k, 2)
  }
  set(cy, Math.round(cx), 'O', 2)
  set(cy, Math.round(cx) - 1, '(', 1)
  set(cy, Math.round(cx) + 1, ')', 1)
  // Rolling counter
  const n = Math.floor(t * 3.7) % 1000000
  const digits = String(n).padStart(6, '0')
  const box = '[ ' + digits.slice(0, 3) + ' ' + digits.slice(3) + ' ]'
  const c0 = Math.round(cx - box.length / 2) + 1
  for (let i = 0; i < box.length; i++)
    set(18, c0 + i, box[i], /[0-9]/.test(box[i]) ? (i >= box.length - 3 ? 2 : 1) : 0)
  return f
}

/** Features: a settings card whose switches flip on and off in a wave. */
function features(t: number): Frame {
  const f = mk()
  const set = (r: number, c: number, k: string, l: Level) => {
    if (r >= 0 && r < H && c >= 0 && c < W) {
      f.ch[r * W + c] = k
      f.lv[r * W + c] = l
    }
  }
  const L = 5,
    R = 38,
    T = 2,
    B = 19
  set(T, L, '╭', 0)
  set(T, R, '╮', 0)
  set(B, L, '╰', 0)
  set(B, R, '╯', 0)
  for (let c = L + 1; c < R; c++) {
    set(T, c, '─', 0)
    set(B, c, '─', 0)
  }
  for (let r = T + 1; r < B; r++) {
    set(r, L, '│', 0)
    set(r, R, '│', 0)
  }
  const names = ['sso', 'api_access', 'audit_log', 'custom_domain']
  names.forEach((name, i) => {
    const r = 5 + i * 4
    const s = Math.sin(t * 0.04 + i * 1.4)
    const p = Math.max(0, Math.min(1, (s + 0.25) / 0.5))
    const on = p > 0.5
    for (let k = 0; k < name.length; k++) set(r, 8 + k, name[k], on ? 2 : 1)
    const sx = 28
    set(r, sx, '(', on ? 1 : 0)
    set(r, sx + 6, ')', on ? 1 : 0)
    const knob = sx + 1 + Math.round(p * 4)
    for (let c = sx + 1; c < sx + 6; c++) set(r, c, on ? '=' : '-', on ? 1 : 0)
    set(r, knob, on ? '●' : '○', on ? 2 : 1)
    if (i < names.length - 1) for (let c = L + 2; c < R - 1; c += 2) set(r + 2, c, '·', 0)
  })
  return f
}

/** Add-ons: a module drifts in and docks onto a space station, then backs off again. */
function addons(t: number): Frame {
  const f = mk()
  const set = (r: number, c: number, k: string, l: Level) => {
    if (r >= 0 && r < H && c >= 0 && c < W && k !== ' ') {
      f.ch[r * W + c] = k
      f.lv[r * W + c] = l
    }
  }
  const stars: [number, number][] = [
    [1, 6],
    [2, 33],
    [4, 41],
    [17, 4],
    [19, 28],
    [20, 12],
    [3, 20],
    [18, 39],
    [15, 24],
  ]
  stars.forEach(([r, c], k) => {
    const v = Math.sin(t * 0.16 + k * 1.7)
    set(r, c, v > 0.6 ? '+' : '.', v > 0.6 ? 2 : 0)
  })
  const station = [
    '  |##|##|        ',
    '  |##|##|        ',
    '  |##|##|        ',
    '     ||          ',
    ' .-----------.   ',
    '[|  o  o  o  |>  ',
    '[|  o  o  o  |>  ',
    " '-----------'   ",
    '     ||          ',
    '  |##|##|        ',
    '  |##|##|        ',
    '  |##|##|        ',
  ]
  station.forEach((s, ri) => {
    for (let c = 0; c < s.length; c++)
      set(5 + ri, 1 + c, s[c], s[c] === 'o' ? 2 : s[c] === '#' ? 0 : 1)
  })
  const cyc = 150,
    p = t % cyc
  const ease = (x: number) => 1 - Math.pow(1 - x, 3)
  let off: number
  if (p < 70) off = 26 * (1 - ease(p / 70))
  else if (p < 110) off = 0
  else off = 26 * Math.pow((p - 110) / 40, 3)
  const mod = ['   .------.  |#|', '<|  ##    |==|#|', '<|  ##    |==|#|', "   '------'  |#|"]
  const mx = 18 + Math.round(off)
  mod.forEach((s, ri) => {
    for (let c = 0; c < s.length; c++) set(9 + ri, mx + c, s[c], s[c] === '#' ? 0 : 1)
  })
  if (p >= 70 && p < 110) {
    const blink = Math.floor(p / 5) % 2 === 0
    set(10, 17, '+', blink ? 2 : 1)
    set(11, 17, '+', blink ? 2 : 1)
    if (p < 78) {
      set(8, 17, '*', 2)
      set(13, 17, '*', 2)
    }
  } else if (off < 14) {
    const r = 10 + (Math.floor(t * 0.5) % 2) // small thruster puffs while moving
    set(r, mx + 16, '-', 1)
  }
  return f
}

/** Wallet: coins drop and spin onto a growing stack. */
function wallet(t: number): Frame {
  const f = mk()
  const set = (r: number, c: number, k: string, l: Level) => {
    if (r >= 0 && r < H && c >= 0 && c < W && k !== ' ') {
      f.ch[r * W + c] = k
      f.lv[r * W + c] = l
    }
  }
  const base = 19,
    per = 24,
    maxN = 9
  for (let c = 4; c < W - 4; c++) set(base + 1, c, '‾', 0)
  const coinRow = (r: number, cx: number, l: Level) => {
    const s = '(=====)'
    for (let i = 0; i < s.length; i++) set(r, cx - 3 + i, s[i], l)
  }
  // Two quiet stacks at the sides
  for (let i = 0; i < 3; i++) coinRow(base - i, 10 + (i % 2), 0)
  for (let i = 0; i < 5; i++) coinRow(base - i, 33 - (i % 2), 0)
  const cyc = per * (maxN + 2)
  const p = t % cyc
  const n = Math.min(maxN, Math.floor(p / per))
  const jit = (i: number) => ((i * 7919) % 3) - 1
  for (let i = 0; i < n; i++) coinRow(base - i, 22 + jit(i), i === n - 1 ? 2 : 1)
  const tau = p - n * per
  if (n < maxN) {
    const landRow = base - n
    const fall = Math.min(1, tau / 16)
    const r = Math.round(-1 + fall * fall * (landRow + 1))
    if (tau < 16) {
      const spin = ['(€)', '(|)', ' | ', '(|)']
      const s = spin[Math.floor(tau / 2) % 4]
      for (let i = 0; i < 3; i++) set(r, 21 + i, s[i], s[i] === '€' ? 2 : 1)
    } else if (tau < 20) {
      set(landRow, 22 + jit(n) - 5, '*', 2)
      set(landRow, 22 + jit(n) + 5, '*', 2)
      coinRow(landRow, 22 + jit(n), 2)
    }
  }
  return f
}

export const scenes: Record<SceneName, (t: number) => Frame> = {
  customers,
  plans,
  invoices,
  events,
  reports,
  subscriptions,
  products,
  metrics,
  features,
  addons,
  wallet,
}

/** Splits a frame into three strings (dim, mid, bright) for three stacked <pre> layers. */
export function toLayers(f: Frame): [string, string, string] {
  const out: [string, string, string] = ['', '', '']
  for (let r = 0; r < H; r++) {
    for (let c = 0; c < W; c++) {
      const i = r * W + c
      for (let k = 0; k < 3; k++) out[k] += f.lv[i] === k ? f.ch[i] : ' '
    }
    if (r < H - 1) {
      out[0] += '\n'
      out[1] += '\n'
      out[2] += '\n'
    }
  }
  return out
}
