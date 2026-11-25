//! When a change takes effect (ADR-0013 §10): the one place a `when` is read and checked against
//! what the change is, which way it goes and what its rules allow.
use super::model::{Direction, When, WhenType};
use crate::v2::problem::Problem;
use chrono::NaiveDate;

/// The change a `when` is sent with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    PlanChange,
    Amendment,
    /// A usage-levels charge moving to another level.
    Level,
    Cancellation,
}

impl ChangeKind {
    fn name(self) -> &'static str {
        match self {
            ChangeKind::PlanChange => "A plan change",
            ChangeKind::Amendment => "An amendment",
            ChangeKind::Level => "A level change",
            ChangeKind::Cancellation => "A cancellation",
        }
    }
}

/// When the change takes effect, checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timing {
    Immediate,
    PeriodEnd,
    On(NaiveDate),
}

impl Timing {
    pub fn kind(self) -> WhenType {
        match self {
            Timing::Immediate => WhenType::Immediate,
            Timing::PeriodEnd => WhenType::PeriodEnd,
            Timing::On(_) => WhenType::Date,
        }
    }
}

/// What the change's rules say of it, known before it is made (a preview of it).
#[derive(Debug, Clone, Copy, Default)]
pub struct Rules {
    /// Which way it goes, when known.
    pub direction: Option<Direction>,
    /// False when it may only take effect at the period's end (a minimum commitment holds it).
    pub period_end_only: bool,
}

/// The timing `when` asks of a change of `kind`, refused at `/when…` when the change cannot take
/// effect then. With no `when`: a level change goes as its direction says; every other change
/// names one.
pub fn timing(when: Option<&When>, kind: ChangeKind, rules: Rules) -> Result<Timing, Problem> {
    let Some(when) = when else {
        return match (kind, rules.direction) {
            (ChangeKind::Level, Some(Direction::Downgrade)) => Ok(Timing::PeriodEnd),
            (ChangeKind::Level, _) => Ok(Timing::Immediate),
            _ => Err(Problem::field(
                "/when",
                "required",
                format!(
                    "{} says when it takes effect: {{\"type\": \"immediate\"}} or \
                     {{\"type\": \"period_end\"}}",
                    kind.name()
                ),
            )),
        };
    };
    let asked = match (when.kind, when.date) {
        (WhenType::Date, Some(date)) => Timing::On(date),
        (WhenType::Date, None) => {
            return Err(Problem::field(
                "/when/date",
                "required",
                "`type: date` takes a `date`",
            ));
        }
        (_, Some(_)) => {
            return Err(Problem::field(
                "/when/date",
                "invalid",
                "A `date` goes with `type: date` only",
            ));
        }
        (WhenType::Immediate, None) => Timing::Immediate,
        (WhenType::PeriodEnd, None) => Timing::PeriodEnd,
    };
    if matches!(asked, Timing::On(_)) && kind != ChangeKind::Cancellation {
        return Err(Problem::when_refused(
            "/when/type",
            format!(
                "{} takes effect now or at the period's end, not on a date",
                kind.name()
            ),
        ));
    }
    if asked == Timing::Immediate && rules.period_end_only {
        return Err(Problem::when_refused(
            "/when/type",
            "A minimum commitment holds this change to the period's end: send \
             {\"type\": \"period_end\"}",
        ));
    }
    match (kind, rules.direction, asked) {
        (
            ChangeKind::PlanChange | ChangeKind::Level,
            Some(Direction::Downgrade),
            Timing::Immediate,
        ) => Err(Problem::when_refused(
            "/when/type",
            format!(
                "{} to a lower one takes effect at the period's end: send \
                     {{\"type\": \"period_end\"}}",
                kind.name()
            ),
        )),
        (ChangeKind::Level, Some(Direction::Upgrade), Timing::PeriodEnd) => {
            Err(Problem::when_refused(
                "/when/type",
                "A higher level applies at once: send {\"type\": \"immediate\"}",
            ))
        }
        _ => Ok(asked),
    }
}

/// A cancellation's date: not before `today` (the past is settled), not after the subscription's
/// own end date, when it has one.
pub fn cancellation_date(
    timing: Timing,
    today: NaiveDate,
    end_date: Option<NaiveDate>,
) -> Result<(), Problem> {
    let Timing::On(date) = timing else {
        return Ok(());
    };
    if date < today {
        return Err(Problem::when_refused(
            "/when/date",
            format!("A cancellation is dated today ({today}) or later"),
        ));
    }
    if let Some(end) = end_date
        && date > end
    {
        return Err(Problem::when_refused(
            "/when/date",
            format!("The subscription ends on {end}: a cancellation is dated on or before it"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn when(kind: WhenType, date: Option<NaiveDate>) -> When {
        When { kind, date }
    }

    #[test]
    fn a_downgrade_takes_effect_at_the_period_end() {
        let downgrade = Rules {
            direction: Some(Direction::Downgrade),
            period_end_only: false,
        };
        let refused = timing(
            Some(&when(WhenType::Immediate, None)),
            ChangeKind::PlanChange,
            downgrade,
        )
        .unwrap_err();
        assert_eq!(refused.code, crate::v2::problem::ProblemCode::WhenRefused);
        assert_eq!(refused.errors[0].path, "/when/type");
        assert_eq!(
            timing(
                Some(&when(WhenType::PeriodEnd, None)),
                ChangeKind::PlanChange,
                downgrade
            )
            .unwrap(),
            Timing::PeriodEnd
        );
        // An amendment may give unused time back now.
        assert_eq!(
            timing(
                Some(&when(WhenType::Immediate, None)),
                ChangeKind::Amendment,
                downgrade
            )
            .unwrap(),
            Timing::Immediate
        );
    }

    #[test]
    fn a_cancellation_is_dated_from_today_to_the_subscription_s_end() {
        let day = |d| NaiveDate::from_ymd_opt(2027, 1, d).unwrap();
        assert!(cancellation_date(Timing::On(day(10)), day(10), None).is_ok());
        assert!(cancellation_date(Timing::On(day(9)), day(10), None).is_err());
        assert!(cancellation_date(Timing::On(day(20)), day(10), Some(day(15))).is_err());
        assert!(cancellation_date(Timing::On(day(15)), day(10), Some(day(15))).is_ok());
        assert!(cancellation_date(Timing::Immediate, day(10), Some(day(1))).is_ok());
    }

    #[test]
    fn a_date_is_a_cancellation_s_only() {
        let date = NaiveDate::from_ymd_opt(2027, 1, 1);
        assert_eq!(
            timing(
                Some(&when(WhenType::Date, date)),
                ChangeKind::Cancellation,
                Rules::default()
            )
            .unwrap(),
            Timing::On(date.unwrap())
        );
        assert!(
            timing(
                Some(&when(WhenType::Date, date)),
                ChangeKind::PlanChange,
                Rules::default()
            )
            .is_err()
        );
        assert!(
            timing(
                Some(&when(WhenType::Date, None)),
                ChangeKind::Cancellation,
                Rules::default()
            )
            .is_err()
        );
        assert!(timing(None, ChangeKind::Cancellation, Rules::default()).is_err());
    }

    #[test]
    fn a_level_goes_as_its_direction_says() {
        let up = Rules {
            direction: Some(Direction::Upgrade),
            period_end_only: false,
        };
        assert_eq!(
            timing(None, ChangeKind::Level, up).unwrap(),
            Timing::Immediate
        );
        assert!(
            timing(
                Some(&when(WhenType::PeriodEnd, None)),
                ChangeKind::Level,
                up
            )
            .is_err()
        );
        let held = Rules {
            direction: Some(Direction::Upgrade),
            period_end_only: true,
        };
        assert!(
            timing(
                Some(&when(WhenType::Immediate, None)),
                ChangeKind::PlanChange,
                held
            )
            .is_err()
        );
    }
}
