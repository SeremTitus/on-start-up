//! Schedule matching for time/date triggers.
//!
//! All times are local wall clock. A scheduled run fires within a short window
//! (2 minutes) after its instant — the scheduler polls every ~15 s — and each
//! instant fires at most once per entry (tracked via `last_fired`).

use chrono::{Datelike, Duration, Local, NaiveDateTime, NaiveTime};

use crate::model::{Entry, Trigger};

const FIRE_WINDOW_MINUTES: i64 = 2;

/// If the entry's schedule matches right now and this instant hasn't fired
/// yet, returns a stamp for the scheduled instant to record after starting.
pub fn due_stamp(entry: &Entry) -> Option<String> {
    let scheduled = scheduled_instant(entry, Local::now().naive_local())?;
    let stamp = scheduled.format("%Y-%m-%dT%H:%M:%S").to_string();
    if entry.last_fired.as_deref() == Some(stamp.as_str()) {
        return None;
    }
    Some(stamp)
}

fn scheduled_instant(entry: &Entry, now: NaiveDateTime) -> Option<NaiveDateTime> {
    let scheduled = match &entry.trigger {
        Trigger::Startup => return None,
        Trigger::Once { date, time } => NaiveDateTime::parse_from_str(
            &format!("{date} {time}"),
            "%Y-%m-%d %H:%M",
        )
        .ok()?,
        Trigger::Daily { time } => {
            now.date()
                .and_time(NaiveTime::parse_from_str(time, "%H:%M").ok()?)
        }
        Trigger::Weekly { days, time } => {
            let weekday = now.weekday().num_days_from_monday() as u8;
            if !days.contains(&weekday) {
                return None;
            }
            now.date()
                .and_time(NaiveTime::parse_from_str(time, "%H:%M").ok()?)
        }
    };

    if now >= scheduled && now < scheduled + Duration::minutes(FIRE_WINDOW_MINUTES) {
        Some(scheduled)
    } else {
        None
    }
}
