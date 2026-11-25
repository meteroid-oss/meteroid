import { PlanOverview, PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'

/** Whether a plan may hold a rank: the server refuses archived and custom plans. */
export const canBeRanked = (plan: PlanOverview) =>
  plan.planStatus !== PlanStatus.ARCHIVED && plan.planType !== PlanType.CUSTOM

/**
 * A product line's ladder, top to bottom. `override` is an order still being saved; a plan
 * listed there but no longer loaded is dropped.
 */
export const ladderOf = (plans: PlanOverview[], override?: string[]): PlanOverview[] => {
  if (override) {
    const byId = new Map(plans.map(plan => [plan.id, plan]))
    return override.flatMap(id => {
      const plan = byId.get(id)
      return plan && canBeRanked(plan) ? [plan] : []
    })
  }
  return plans
    .filter(plan => plan.selfServiceRank !== undefined && canBeRanked(plan))
    .sort(byRank)
}

/** Rank order as the server reads it: plans sharing a rank (older data) by age, then id. */
export const byRank = (a: PlanOverview, b: PlanOverview): number =>
  (a.selfServiceRank ?? 0) - (b.selfServiceRank ?? 0) ||
  compare(a.createdAt, b.createdAt) ||
  compare(a.id, b.id)

const compare = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0)

/**
 * The whole ladder after moving `id` among the `visible` plans to `target`'s place. Plans
 * hidden by the filters keep their slots, so reordering a filtered view never drops them.
 */
export const moveWithin = (
  ladder: string[],
  visible: string[],
  id: string,
  target: string
): string[] => {
  const from = visible.indexOf(id)
  const to = visible.indexOf(target)
  if (from < 0 || to < 0 || from === to) return ladder
  const moved = [...visible]
  moved.splice(to, 0, ...moved.splice(from, 1))
  const shown = new Set(visible)
  let next = 0
  return ladder.map(planId => (shown.has(planId) ? moved[next++] : planId))
}
