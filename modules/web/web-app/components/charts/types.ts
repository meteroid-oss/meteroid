export type MetricUnit = 'currency' | 'count' | 'percent'
/** Whether a rising value is good, bad or directionless; drives trend/delta colouring. */
export type Polarity = 'higher_is_better' | 'lower_is_better' | 'neutral'

/** What a chart needs to know about the series it draws. */
export interface ChartSpec {
  label: string
  /** How values are formatted and scaled (currency is minor units -> units). */
  unit: MetricUnit
  /** Headline KPI: sum a flow over the range, or take the latest for a stock. */
  summary: 'sum' | 'latest'
  /** When set, the series is split by this dimension (stacked breakdown). */
  groupBy?: string
  /**
   * Chart style. `line` for cumulative balances/stocks, `bar` for per-period
   * flows, `stacked` for grouped breakdowns. Defaults: a `groupBy` forces
   * `stacked`, a `summary: 'latest'` stock defaults to `line`, otherwise `bar`.
   */
  chart?: 'line' | 'area' | 'bar' | 'stacked'
}

export interface ForecastPoint {
  /** ISO date (YYYY-MM-DD) of the projected window, beyond the last actual. */
  windowStart: string
  /** Projected raw value (same unit as the input points). */
  value: number
  /** Lower / upper edge of the confidence band, same unit. */
  lower: number
  upper: number
}
