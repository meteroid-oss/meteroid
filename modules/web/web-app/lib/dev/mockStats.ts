import { create } from '@bufbuild/protobuf'
import { timestampFromDate } from '@bufbuild/protobuf/wkt'

import { MRRMovementType } from '@/rpc/api/stats/v1/models_pb'

import type { DescMessage, MessageInitShape } from '@bufbuild/protobuf'
import type { Interceptor, UnaryResponse } from '@connectrpc/connect'

/**
 * Development only: sample data for the stats service (Home's figures and charts), for setups
 * where it can't answer, such as the lite docker setup without metering. Off by default; turn it
 * on with `?mockStats=on` in the URL (remembered in this browser) and off with `?mockStats=off`.
 * The data is derived from the date alone, so it is the same on every reload and the "previous
 * period" comparisons come out believable.
 */

const STORAGE_KEY = 'meteroid:mock-stats'
const STATS_SERVICE = 'meteroid.api.stats.v1.StatsService'

const mockStatsEnabled = (): boolean => {
  if (!import.meta.env.DEV || typeof window === 'undefined') return false
  const fromUrl = new URLSearchParams(window.location.search).get('mockStats')
  if (fromUrl === 'on' || fromUrl === 'off') window.localStorage.setItem(STORAGE_KEY, fromUrl)
  return window.localStorage.getItem(STORAGE_KEY) === 'on'
}

// --- deterministic sample data -------------------------------------------------------------

const DAY_MS = 86_400_000
const dayNumber = (date: Date) => Math.floor(date.getTime() / DAY_MS)
const iso = (date: Date) => date.toISOString().slice(0, 10)
// A stable pseudo-random value in [0, 1) for a day (and a salt, for independent series).
const noise = (day: number, salt = 0) => {
  const v = Math.sin(day * 12.9898 + salt * 78.233) * 43758.5453
  return v - Math.floor(v)
}

interface DateShape {
  year?: number
  month?: number
  day?: number
}
const fromDateMessage = (d: DateShape | undefined, fallback: Date) =>
  d?.year ? new Date(Date.UTC(d.year, (d.month ?? 1) - 1, d.day ?? 1)) : fallback

const daysBetween = (from: Date, to: Date) => {
  const days: Date[] = []
  for (let t = dayNumber(from); t <= dayNumber(to); t++) days.push(new Date(t * DAY_MS))
  return days
}

const today = () => new Date(dayNumber(new Date()) * DAY_MS)
// Growth is measured from a fixed day, so a window's numbers don't depend on when it starts.
const ORIGIN = dayNumber(new Date(Date.UTC(2024, 0, 1)))

/** Revenue billed on a day, in cents: a slow upward trend, weekly rhythm and some noise. */
const dailyRevenueCents = (day: number) => {
  const trend = 900 + (day - ORIGIN) * 1.6
  const weekday = [0.6, 1.1, 1.15, 1.1, 1.05, 1, 0.65][((day % 7) + 7) % 7]
  return Math.round(trend * weekday * (0.75 + noise(day) * 0.5) * 100)
}

/** Total MRR on a day, in cents: steps up with new business, a little churn now and then. */
const mrrCents = (day: number) => {
  const months = (day - ORIGIN) / 30
  return Math.round((28_000 + months * 1_150 + Math.sin(months) * 600) * 100)
}

const breakdownFor = (day: number) => {
  const stat = (salt: number, scale: number) => ({
    count: BigInt(Math.round(noise(day, salt) * 4)),
    value: BigInt(Math.round(noise(day, salt + 1) * scale * 100)),
  })
  return {
    newBusiness: stat(1, 900),
    expansion: stat(3, 400),
    contraction: stat(5, 150),
    churn: stat(7, 250),
    reactivation: stat(9, 80),
    netNewMrr: BigInt(Math.round((noise(day, 11) * 900 - 200) * 100)),
    totalNetMrr: BigInt(mrrCents(day)),
  }
}

const CUSTOMERS = [
  ['Clearbit', 'Professional'],
  ['Figma', 'Enterprise'],
  ['Jordan', 'Enterprise'],
  ['Apple', 'Enterprise'],
  ['Linear', 'Professional'],
  ['Vercel', 'Growth'],
  ['Notion', 'Professional'],
  ['Raycast', 'Starter'],
] as const

// --- one builder per stats method (by proto method name) ---------------------------------------

type Builder = (input: Record<string, unknown>) => MessageInitShape<DescMessage>

const BUILDERS: Record<string, Builder> = {
  GeneralStats: () => {
    const day = dayNumber(today())
    const last30 = daysBetween(new Date((day - 29) * DAY_MS), today())
    const current = last30.reduce((sum, d) => sum + dailyRevenueCents(dayNumber(d)), 0)
    return {
      totalNetRevenue: {
        trend: {
          current: BigInt(current),
          changeAmount: BigInt(Math.round(current * 0.121)),
          changePercent: 12.1,
          positiveIsGood: true,
        },
      },
      totalActiveSubscriptions: { count: 661n },
      pendingInvoices: { count: 32n, valueCents: 1_842_000n },
      signups: { count: 63n },
      trialConversion: { ratePercent: 34.5 },
      totalMrr: { valueCents: BigInt(mrrCents(day)) },
    }
  },

  TotalRevenueChart: input => {
    const to = fromDateMessage(input.endDate as DateShape, today())
    const from = fromDateMessage(input.startDate as DateShape, new Date(to.getTime() - 90 * DAY_MS))
    let total = 0
    const data = daysBetween(from, to).map(date => {
      const daily = dailyRevenueCents(dayNumber(date))
      total += daily
      return { x: iso(date), revenue: BigInt(total), dailyRevenue: BigInt(daily) }
    })
    return { series: [{ name: 'Revenue', code: 'total_revenue', data }] }
  },

  TotalMrrChart: input => {
    const to = fromDateMessage(input.endDate as DateShape, today())
    const from = fromDateMessage(input.startDate as DateShape, new Date(to.getTime() - 90 * DAY_MS))
    const data = daysBetween(from, to).map(date => ({
      x: iso(date),
      data: breakdownFor(dayNumber(date)),
    }))
    return { series: [{ name: 'MRR', code: 'total_mrr', data }] }
  },

  MrrBreakdown: () => ({
    mmrBreakdown: {
      newBusiness: { count: 9n, value: 640_000n },
      expansion: { count: 5n, value: 248_000n },
      reactivation: { count: 2n, value: 40_000n },
      contraction: { count: 3n, value: 52_000n },
      churn: { count: 1n, value: 18_000n },
      netNewMrr: 858_000n,
      totalNetMrr: BigInt(mrrCents(dayNumber(today()))),
    },
  }),

  MrrLog: () => {
    const types = [
      MRRMovementType.NEW_BUSINESS,
      MRRMovementType.EXPANSION,
      MRRMovementType.NEW_BUSINESS,
      MRRMovementType.CONTRACTION,
      MRRMovementType.EXPANSION,
      MRRMovementType.REACTIVATION,
      MRRMovementType.NEW_BUSINESS,
      MRRMovementType.CHURN,
    ]
    return {
      entries: CUSTOMERS.map(([name, plan], i) => {
        const date = new Date(today().getTime() - i * 2 * DAY_MS)
        return {
          createdAt: timestampFromDate(date),
          customerId: `mock-customer-${i}`,
          customerName: name,
          subscriptionId: `mock-subscription-${i}`,
          planName: plan,
          description: '',
          mrrType: types[i],
          appliesTo: {
            year: date.getUTCFullYear(),
            month: date.getUTCMonth() + 1,
            day: date.getUTCDate(),
          },
          mrrChange: BigInt([64_000, 24_000, 99_000, 12_000, 30_000, 4_000, 49_000, 18_000][i]),
          currency: 'EUR',
        }
      }),
    }
  },

  TopRevenueByCustomer: input => ({
    revenueByCustomer: CUSTOMERS.slice(0, Number(input.count) || 5).map(([name], i) => ({
      customerName: name,
      customerId: `mock-customer-${i}`,
      revenueYtd: BigInt(Math.round((52_000 / (i + 1.4)) * 100)),
      revenueAllTime: BigInt(Math.round((131_000 / (i + 1.4)) * 100)),
    })),
  }),

  SignupSparkline: () => {
    let total = 0
    const data = daysBetween(new Date(today().getTime() - 29 * DAY_MS), today()).map(date => {
      const delta = Math.round(noise(dayNumber(date), 21) * 5)
      total += delta
      return { x: iso(date), total: BigInt(total), delta: BigInt(delta) }
    })
    return { series: { name: 'New signups', code: 'new_signups', data } }
  },

  TrialConversionRateSparkline: () => {
    const months = Array.from({ length: 12 }, (_, i) => {
      const d = today()
      return new Date(Date.UTC(d.getUTCFullYear(), d.getUTCMonth() - 11 + i, 1))
    })
    return {
      series: [
        {
          name: 'Conversion rate',
          code: 'conversion_rate',
          data: months.map((date, i) => ({
            x: iso(date),
            conversionRate: 24 + i * 0.9 + noise(dayNumber(date), 31) * 4,
            conversions: BigInt(6 + Math.round(noise(dayNumber(date), 33) * 6)),
          })),
        },
      ],
      metadata: months.map((date, i) => ({ x: iso(date), totalTrials: BigInt(22 + i) })),
    }
  },
}

/** Answers stats requests with the sample data above while mock stats are on (dev only). */
export const mockStatsInterceptor: Interceptor = next => async req => {
  const build = BUILDERS[req.method.name]
  if (req.stream || req.service.typeName !== STATS_SERVICE || !build || !mockStatsEnabled()) {
    return next(req)
  }
  const response: UnaryResponse = {
    stream: false,
    service: req.service,
    method: req.method,
    header: new Headers(),
    trailer: new Headers(),
    message: create(req.method.output, build(req.message as Record<string, unknown>)),
  }
  return response as Awaited<ReturnType<typeof next>>
}
