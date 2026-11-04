//! Pure planner for payment reminder ladders: every function takes `now`, nothing reads a clock
//! or the database. Steps are identified by `offset_days` from the anchor and the cursor is the
//! last **executed** offset, so step-list edits can never re-fire an executed step.

use crate::domain::payment_reminders::SendWindow;
use chrono::{
    Days, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike,
};

/// A step older than this at enrolment is skipped; a ladder whose every step is older never enrols.
pub const ENROLL_WINDOW_HOURS: i64 = 24;
/// Steps execute only inside `[send_hour, send_hour + 6h)` local.
pub const SEND_WINDOW_HOURS: u32 = 6;
/// No reminder step executes within this long after any customer email for the invoice.
pub const COOLDOWN_HOURS: i64 = 20;

const DST_GAP_STEP_MINUTES: i64 = 15;
const DST_GAP_MAX_STEPS: usize = 12;

/// `max(due date, finalization date in the entity zone)`. `due_at` is a naive midnight, so the
/// calendar due date is the same in every zone.
pub fn anchor_date(
    due_at: NaiveDateTime,
    finalized_at: NaiveDateTime,
    w: &SendWindow,
) -> NaiveDate {
    due_at
        .date()
        .max(w.tz.from_utc_datetime(&finalized_at).date_naive())
}

/// The send hour of `date` in the entity zone, as naive UTC. A DST gap rolls forward to the
/// first valid instant (15-minute steps, at most 3 h); an overlap takes the earlier instant.
fn local_slot(date: NaiveDate, w: &SendWindow) -> NaiveDateTime {
    let time = NaiveTime::from_hms_opt(u32::from(w.hour), 0, 0).unwrap_or_default();
    let mut local = date.and_time(time);
    for _ in 0..=DST_GAP_MAX_STEPS {
        match w.tz.from_local_datetime(&local) {
            LocalResult::Single(t) => return t.naive_utc(),
            LocalResult::Ambiguous(a, b) => return a.min(b).naive_utc(),
            LocalResult::None => local += Duration::minutes(DST_GAP_STEP_MINUTES),
        }
    }
    date.and_time(time)
}

/// Local send hour on `anchor + offset_days`, as naive UTC.
pub fn fire_time(anchor: NaiveDate, offset_days: i32, w: &SendWindow) -> NaiveDateTime {
    let date = if offset_days >= 0 {
        anchor.checked_add_days(Days::new(offset_days.unsigned_abs().into()))
    } else {
        anchor.checked_sub_days(Days::new(offset_days.unsigned_abs().into()))
    }
    .unwrap_or(anchor);
    local_slot(date, w)
}

/// A step that would fire before the invoice existed is never sent (I3).
pub fn schedulable(fire: NaiveDateTime, finalized_at: NaiveDateTime) -> bool {
    fire >= finalized_at
}

pub fn in_send_window(now: NaiveDateTime, w: &SendWindow) -> bool {
    let hour = w.tz.from_utc_datetime(&now).hour();
    (hour + 24 - u32::from(w.hour)) % 24 < SEND_WINDOW_HOURS
}

/// Today's local slot when it is still ahead of `now`, else tomorrow's.
pub fn next_send_slot(now: NaiveDateTime, w: &SendWindow) -> NaiveDateTime {
    ceil_to_send_slot(now + Duration::seconds(1), w)
}

/// The first local send slot at or after `t`.
pub fn ceil_to_send_slot(t: NaiveDateTime, w: &SendWindow) -> NaiveDateTime {
    let today = w.tz.from_utc_datetime(&t).date_naive();
    let slot = local_slot(today, w);
    if slot >= t {
        return slot;
    }
    local_slot(today.checked_add_days(Days::new(1)).unwrap_or(today), w)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnrollmentPlan {
    /// Last offset treated as executed (stale steps skipped); `None` = start at the top.
    pub cursor: Option<i32>,
    /// `None` = nothing left to schedule (enrolled, complete).
    pub next_reminder_at: Option<NaiveDateTime>,
}

fn schedulable_offsets(
    steps: &[i32],
    anchor: NaiveDate,
    finalized_at: NaiveDateTime,
    w: &SendWindow,
) -> Vec<(i32, NaiveDateTime)> {
    steps
        .iter()
        .map(|&o| (o, fire_time(anchor, o, w)))
        .filter(|(_, fire)| schedulable(*fire, finalized_at))
        .collect()
}

/// Where a newly eligible invoice enters the ladder. `None` = every schedulable step is older
/// than the enrolment window (`LADDER_STALE`). Zero steps ⇒ enrolled and complete.
/// `steps` must be sorted ascending.
pub fn plan_enrollment(
    steps: &[i32],
    anchor: NaiveDate,
    finalized_at: NaiveDateTime,
    now: NaiveDateTime,
    w: &SendWindow,
) -> Option<EnrollmentPlan> {
    if steps.is_empty() {
        return Some(EnrollmentPlan {
            cursor: None,
            next_reminder_at: None,
        });
    }
    let window_start = now - Duration::hours(ENROLL_WINDOW_HOURS);
    let schedulable = schedulable_offsets(steps, anchor, finalized_at, w);
    let (first_live, fire) = schedulable
        .iter()
        .find(|(_, fire)| *fire > window_start)
        .copied()?;
    let cursor = schedulable
        .iter()
        .take_while(|(o, _)| *o < first_live)
        .last()
        .map(|(o, _)| *o);
    Some(EnrollmentPlan {
        cursor,
        next_reminder_at: Some(fire),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionPlan {
    ExecuteStep {
        offset_days: i32,
        next: Option<NaiveDateTime>,
    },
    /// Nothing executes now; re-stamp `next_reminder_at` without side effects.
    Reschedule(NaiveDateTime),
    Complete,
}

/// What a claimed invoice (`next_reminder_at <= now`) does. Only the **last** elapsed step
/// executes (catch-up never bursts), and only inside the send window and outside the cooldown.
/// `steps` must be sorted ascending.
pub fn plan_execution(
    steps: &[i32],
    anchor: NaiveDate,
    finalized_at: NaiveDateTime,
    cursor: Option<i32>,
    now: NaiveDateTime,
    last_customer_email_at: Option<NaiveDateTime>,
    w: &SendWindow,
) -> ExecutionPlan {
    let remaining: Vec<(i32, NaiveDateTime)> = schedulable_offsets(steps, anchor, finalized_at, w)
        .into_iter()
        .filter(|(o, _)| cursor.is_none_or(|c| *o > c))
        .collect();
    let Some(&(first_offset, first_fire)) = remaining.first() else {
        return ExecutionPlan::Complete;
    };
    let Some(&(exec, _)) = remaining.iter().rfind(|(_, fire)| *fire <= now) else {
        debug_assert!(first_fire > now, "{first_offset}");
        return ExecutionPlan::Reschedule(first_fire);
    };
    let cooling_down = last_customer_email_at
        .is_some_and(|t| now - t < Duration::hours(COOLDOWN_HOURS) && t <= now);
    if !in_send_window(now, w) || cooling_down {
        return ExecutionPlan::Reschedule(next_send_slot(now, w));
    }
    let next = remaining
        .iter()
        .find(|(o, _)| *o > exec)
        .map(|(_, fire)| *fire);
    ExecutionPlan::ExecuteStep {
        offset_days: exec,
        next,
    }
}

/// Restart: the cursor sits just before the last elapsed schedulable step, whose (past) fire
/// time is stamped so the next sweep executes it at the next slot. No elapsed step ⇒ the first
/// schedulable fire; no schedulable step ⇒ complete.
pub fn plan_restart(
    steps: &[i32],
    anchor: NaiveDate,
    finalized_at: NaiveDateTime,
    now: NaiveDateTime,
    w: &SendWindow,
) -> EnrollmentPlan {
    let schedulable = schedulable_offsets(steps, anchor, finalized_at, w);
    let Some(&(first, first_fire)) = schedulable.first() else {
        return EnrollmentPlan {
            cursor: None,
            next_reminder_at: None,
        };
    };
    let Some(&(last_elapsed, fire)) = schedulable.iter().rfind(|(_, f)| *f <= now) else {
        debug_assert!(first_fire > now, "{first}");
        return EnrollmentPlan {
            cursor: None,
            next_reminder_at: Some(first_fire),
        };
    };
    let cursor = schedulable
        .iter()
        .take_while(|(o, _)| *o < last_elapsed)
        .last()
        .map(|(o, _)| *o);
    EnrollmentPlan {
        cursor,
        next_reminder_at: Some(fire),
    }
}

/// The next fire after `cursor` for a settings change: the next schedulable step's fire time,
/// or the next send slot when that step already elapsed. `None` = nothing left.
pub fn plan_restamp(
    steps: &[i32],
    anchor: NaiveDate,
    finalized_at: NaiveDateTime,
    cursor: Option<i32>,
    now: NaiveDateTime,
    w: &SendWindow,
) -> Option<NaiveDateTime> {
    let (_, fire) = schedulable_offsets(steps, anchor, finalized_at, w)
        .into_iter()
        .find(|(o, _)| cursor.is_none_or(|c| *o > c))?;
    Some(if fire <= now {
        next_send_slot(now, w)
    } else {
        fire
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Tz;

    fn paris() -> SendWindow {
        SendWindow {
            tz: chrono_tz::Europe::Paris,
            hour: 9,
        }
    }

    fn utc() -> SendWindow {
        SendWindow {
            tz: chrono_tz::UTC,
            hour: 9,
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        date(y, m, d).and_hms_opt(h, min, 0).unwrap()
    }

    /// March 2026 in UTC: `day` at `hour`.
    fn mar(day: u32, hour: u32) -> NaiveDateTime {
        dt(2026, 3, day, hour, 0)
    }

    #[test]
    fn anchor_is_the_later_of_due_and_finalized_in_entity_zone() {
        let w = paris();
        // Finalized 23:30 UTC on the 9th is already the 10th in Paris.
        assert_eq!(
            anchor_date(mar(9, 0), dt(2026, 3, 9, 23, 30), &w),
            date(2026, 3, 10)
        );
        // Due date later than finalization: the due date wins.
        assert_eq!(anchor_date(mar(20, 0), mar(2, 12), &w), date(2026, 3, 20));
        // Backdated invoice finalized after its due date: finalization wins.
        assert_eq!(anchor_date(mar(2, 0), mar(10, 12), &w), date(2026, 3, 10));
    }

    #[test]
    fn fire_time_is_local_send_hour_on_anchor_date() {
        let w = paris();
        assert_eq!(fire_time(date(2026, 3, 28), 0, &w), mar(28, 8));
        assert_eq!(fire_time(date(2026, 3, 30), 0, &w), mar(30, 7));
        assert_eq!(fire_time(date(2026, 3, 28), 2, &w), mar(30, 7));
        assert_eq!(fire_time(date(2026, 3, 30), -3, &w), mar(27, 8));
    }

    #[test]
    fn fire_time_dst_gap_rolls_forward_without_panic() {
        // Paris 2026-03-29: 02:00 local does not exist; 03:00 CEST = 01:00Z.
        let w = SendWindow {
            tz: chrono_tz::Europe::Paris,
            hour: 2,
        };
        assert_eq!(fire_time(date(2026, 3, 29), 0, &w), mar(29, 1));

        // Troll station jumps two hours (UTC+0 → UTC+2) on the same day: 01:00 → 03:00 local.
        let troll: Tz = "Antarctica/Troll".parse().expect("zone");
        let w = SendWindow { tz: troll, hour: 1 };
        let fire = fire_time(date(2026, 3, 29), 0, &w);
        assert_eq!(fire, mar(29, 1), "first valid instant after the gap");
        let w = SendWindow { tz: troll, hour: 2 };
        assert_eq!(fire_time(date(2026, 3, 29), 0, &w), mar(29, 1));
    }

    #[test]
    fn fire_time_dst_overlap_takes_the_earlier_instant() {
        // Paris 2025-10-26: 02:00 local happens twice; the CEST one is 00:00Z.
        let w = SendWindow {
            tz: chrono_tz::Europe::Paris,
            hour: 2,
        };
        assert_eq!(fire_time(date(2025, 10, 26), 0, &w), dt(2025, 10, 26, 0, 0));
    }

    #[test]
    fn fire_time_utc_when_no_timezone() {
        assert_eq!(
            fire_time(date(2026, 3, 29), 7, &utc()),
            dt(2026, 4, 5, 9, 0)
        );
    }

    #[test]
    fn send_window_and_next_slot() {
        let w = paris();
        assert!(in_send_window(mar(10, 8), &w)); // 09:00 Paris
        assert!(in_send_window(mar(10, 13), &w)); // 14:00 Paris
        assert!(!in_send_window(mar(10, 14), &w)); // 15:00 Paris
        assert!(!in_send_window(mar(10, 2), &w)); // 03:00 Paris
        assert_eq!(next_send_slot(mar(10, 2), &w), mar(10, 8));
        assert_eq!(next_send_slot(mar(10, 8), &w), mar(11, 8));
        assert_eq!(next_send_slot(mar(10, 14), &w), mar(11, 8));
        assert_eq!(ceil_to_send_slot(mar(10, 8), &w), mar(10, 8));
        assert_eq!(ceil_to_send_slot(mar(10, 9), &w), mar(11, 8));

        let late = SendWindow {
            tz: chrono_tz::UTC,
            hour: 22,
        };
        assert!(in_send_window(mar(10, 1), &late));
        assert!(!in_send_window(mar(10, 4), &late));
    }

    #[test]
    fn fresh_invoice_enrolls_at_first_step() {
        let w = utc();
        let plan =
            plan_enrollment(&[-3, 0, 7], date(2026, 3, 10), mar(1, 12), mar(2, 0), &w).unwrap();
        assert_eq!(plan.cursor, None);
        assert_eq!(plan.next_reminder_at, Some(mar(7, 9)));
    }

    #[test]
    fn enrollment_skips_stale_steps_and_joins_mid_ladder() {
        let w = utc();
        // -3 (7th) and 0 (10th) are stale on the 15th; +7 (17th) is live.
        let plan =
            plan_enrollment(&[-3, 0, 7], date(2026, 3, 10), mar(1, 12), mar(15, 0), &w).unwrap();
        assert_eq!(plan.cursor, Some(0));
        assert_eq!(plan.next_reminder_at, Some(mar(17, 9)));
    }

    #[test]
    fn enrollment_within_24h_window_keeps_a_just_missed_step() {
        let w = utc();
        // Day 0 fired at 09:00 on the 10th; enrolling at 20:00 the same day keeps it.
        let plan =
            plan_enrollment(&[0, 7], date(2026, 3, 10), mar(1, 12), mar(10, 20), &w).unwrap();
        assert_eq!(plan.cursor, None);
        assert_eq!(plan.next_reminder_at, Some(mar(10, 9)));
    }

    #[test]
    fn fully_stale_ladder_returns_none() {
        let w = utc();
        assert!(plan_enrollment(&[0, 3], date(2026, 3, 1), mar(1, 0), mar(20, 0), &w).is_none());
        // Every step precedes finalization: nothing can ever fire either.
        assert!(
            plan_enrollment(&[-3, 0], date(2026, 3, 10), mar(10, 14), mar(10, 15), &w).is_none()
        );
    }

    #[test]
    fn zero_step_sequence_enrolls_complete() {
        let plan = plan_enrollment(&[], date(2026, 3, 10), mar(1, 12), mar(2, 0), &utc()).unwrap();
        assert_eq!(
            plan,
            EnrollmentPlan {
                cursor: None,
                next_reminder_at: None
            }
        );
    }

    #[test]
    fn step_before_finalization_is_not_schedulable() {
        let w = utc();
        // Net-0 invoice finalized at 14:00 on its due date: day 0 (09:00) precedes finalization.
        let plan =
            plan_enrollment(&[0, 7], date(2026, 3, 10), mar(10, 14), mar(10, 15), &w).unwrap();
        assert_eq!(
            plan.cursor, None,
            "an unschedulable step is not an executed one"
        );
        assert_eq!(plan.next_reminder_at, Some(mar(17, 9)));
        assert_eq!(
            plan_execution(
                &[0, 7],
                date(2026, 3, 10),
                mar(10, 14),
                None,
                mar(10, 15),
                None,
                &w
            ),
            ExecutionPlan::Reschedule(mar(17, 9))
        );
    }

    #[test]
    fn executes_due_step_and_schedules_the_next() {
        let w = utc();
        let plan = plan_execution(
            &[3, 7],
            date(2026, 3, 10),
            mar(1, 12),
            None,
            mar(13, 9),
            None,
            &w,
        );
        assert_eq!(
            plan,
            ExecutionPlan::ExecuteStep {
                offset_days: 3,
                next: Some(mar(17, 9)),
            }
        );
        let last = plan_execution(
            &[3, 7],
            date(2026, 3, 10),
            mar(1, 12),
            Some(3),
            mar(17, 10),
            None,
            &w,
        );
        assert_eq!(
            last,
            ExecutionPlan::ExecuteStep {
                offset_days: 7,
                next: None,
            }
        );
    }

    #[test]
    fn catch_up_executes_only_the_last_elapsed_step() {
        let w = utc();
        // 30 days late with no cursor: 0, 7, 14 elapsed, 21 elapsed too; 28 in the future.
        let plan = plan_execution(
            &[0, 7, 14, 21, 28],
            date(2026, 3, 1),
            dt(2026, 2, 20, 0, 0),
            None,
            mar(25, 10),
            None,
            &w,
        );
        assert_eq!(
            plan,
            ExecutionPlan::ExecuteStep {
                offset_days: 21,
                next: Some(mar(29, 9)),
            }
        );
    }

    #[test]
    fn catch_up_outside_send_window_reschedules_to_next_slot() {
        let w = paris();
        // 03:00 local on the 15th, step 0 (10th) elapsed: wait for 09:00 local.
        let plan = plan_execution(
            &[0, 7],
            date(2026, 3, 10),
            mar(1, 12),
            None,
            mar(15, 2),
            None,
            &w,
        );
        assert_eq!(plan, ExecutionPlan::Reschedule(mar(15, 8)));
    }

    #[test]
    fn cooldown_after_customer_email_reschedules() {
        let w = utc();
        let plan = plan_execution(
            &[0, 7],
            date(2026, 3, 10),
            mar(1, 12),
            None,
            mar(10, 9),
            Some(mar(10, 1)),
            &w,
        );
        assert_eq!(plan, ExecutionPlan::Reschedule(mar(11, 9)));
        // An email older than the cooldown does not hold the step.
        let plan = plan_execution(
            &[0, 7],
            date(2026, 3, 10),
            mar(1, 12),
            None,
            mar(10, 9),
            Some(mar(9, 1)),
            &w,
        );
        assert!(matches!(
            plan,
            ExecutionPlan::ExecuteStep { offset_days: 0, .. }
        ));
    }

    #[test]
    fn edit_pushing_steps_out_reschedules_without_side_effects() {
        let w = utc();
        // The step at +3 was replaced by +10 while the claim was pending.
        let plan = plan_execution(
            &[10],
            date(2026, 3, 10),
            mar(1, 12),
            None,
            mar(13, 9),
            None,
            &w,
        );
        assert_eq!(plan, ExecutionPlan::Reschedule(mar(20, 9)));
    }

    #[test]
    fn trailing_step_removal_completes_without_a_burst() {
        let w = utc();
        let plan = plan_execution(
            &[3],
            date(2026, 3, 10),
            mar(1, 12),
            Some(7),
            mar(17, 9),
            None,
            &w,
        );
        assert_eq!(plan, ExecutionPlan::Complete);
    }

    #[test]
    fn cursor_is_strictly_monotonic() {
        let w = utc();
        // Inserting a step behind the cursor never executes it.
        let plan = plan_execution(
            &[0, 3, 7],
            date(2026, 3, 10),
            mar(1, 12),
            Some(3),
            mar(20, 9),
            None,
            &w,
        );
        assert_eq!(
            plan,
            ExecutionPlan::ExecuteStep {
                offset_days: 7,
                next: None,
            }
        );
        let plan = plan_execution(
            &[0, 1, 2],
            date(2026, 3, 10),
            mar(1, 12),
            Some(2),
            mar(20, 9),
            None,
            &w,
        );
        assert_eq!(plan, ExecutionPlan::Complete);
    }

    #[test]
    fn timezone_change_never_refires_executed_step() {
        let paris = paris();
        let tokyo = SendWindow {
            tz: chrono_tz::Asia::Tokyo,
            hour: 9,
        };
        // Step 0 executed at 09:00 Paris; the entity moves to Tokyo where 09:00 was hours earlier.
        let plan = plan_execution(
            &[0, 7],
            date(2026, 3, 10),
            mar(1, 12),
            Some(0),
            mar(10, 9),
            None,
            &tokyo,
        );
        assert_eq!(
            plan,
            ExecutionPlan::Reschedule(fire_time(date(2026, 3, 10), 7, &tokyo))
        );
        assert_eq!(
            plan_restamp(
                &[0, 7],
                date(2026, 3, 10),
                mar(1, 12),
                Some(0),
                mar(10, 9),
                &tokyo
            ),
            Some(mar(17, 0))
        );
        // The other way round the pending step re-derives to the later local hour, once.
        assert_eq!(
            plan_restamp(
                &[0, 7],
                date(2026, 3, 10),
                mar(1, 12),
                None,
                mar(9, 0),
                &paris
            ),
            Some(mar(10, 8))
        );
        // An elapsed pending step lands on the next slot, never in the past.
        assert_eq!(
            plan_restamp(
                &[0, 7],
                date(2026, 3, 10),
                mar(1, 12),
                None,
                mar(10, 12),
                &paris
            ),
            Some(mar(11, 8))
        );
    }

    #[test]
    fn restart_targets_last_elapsed_step() {
        let w = utc();
        let plan = plan_restart(
            &[0, 7, 14],
            date(2026, 3, 1),
            dt(2026, 2, 20, 0, 0),
            mar(12, 10),
            &w,
        );
        assert_eq!(plan.cursor, Some(0));
        assert_eq!(plan.next_reminder_at, Some(mar(8, 9)));
        // Nothing elapsed yet: start at the top.
        let plan = plan_restart(&[0, 7], date(2026, 3, 10), mar(1, 12), mar(2, 0), &w);
        assert_eq!(plan.cursor, None);
        assert_eq!(plan.next_reminder_at, Some(mar(10, 9)));
        // Nothing schedulable at all: complete.
        let plan = plan_restart(&[-3], date(2026, 3, 10), mar(10, 14), mar(20, 0), &w);
        assert_eq!(plan.next_reminder_at, None);
    }

    #[test]
    fn restart_on_stale_ladder_still_executes_one_step() {
        let w = utc();
        let anchor = date(2026, 1, 1);
        let finalized = dt(2025, 12, 20, 0, 0);
        let now = mar(20, 10);
        assert!(plan_enrollment(&[0, 7, 14], anchor, finalized, now, &w).is_none());
        let plan = plan_restart(&[0, 7, 14], anchor, finalized, now, &w);
        assert_eq!(plan.cursor, Some(7));
        assert_eq!(plan.next_reminder_at, Some(dt(2026, 1, 15, 9, 0)));
        let exec = plan_execution(&[0, 7, 14], anchor, finalized, plan.cursor, now, None, &w);
        assert_eq!(
            exec,
            ExecutionPlan::ExecuteStep {
                offset_days: 14,
                next: None,
            }
        );
    }
}
