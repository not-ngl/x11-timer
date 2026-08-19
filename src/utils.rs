use std::time::Duration;

use crossterm::style::Color;

use crate::timer_state::{TimerMode, TimerState};
use crate::timer::ComparisonMode;

pub enum Sign {
    Positive,
    Negative,
}

impl From<bool> for Sign {
    fn from(positive: bool) -> Self {
        if positive {
            Self::Positive
        } else {
            Self::Negative
        }
    }
}

pub enum Prefix {
    NoneOrMinus,
    PlusOrMinus,
}

pub fn format_duration(duration: Duration, ms_digits: i32, sign: Sign, prefix: Prefix) -> String {
    let minute_sec = 60;
    let hour_sec = minute_sec * 60;
    let day_sec = hour_sec * 24;

    let duration_secs = duration.as_secs();
    let days = duration_secs / day_sec;
    let hours = (duration_secs % day_sec) / hour_sec;
    let minutes = (duration_secs % hour_sec) / minute_sec;
    let seconds = duration_secs % minute_sec;
    let milliseconds = duration.subsec_millis();

    let prefix_str = match (sign, prefix) {
        (Sign::Positive, Prefix::PlusOrMinus) => "+",
        (Sign::Positive, Prefix::NoneOrMinus) => "",
        (Sign::Negative, _) => "-",
    };

    let s = match (days, hours, minutes, seconds, milliseconds) {
        (0, 0, 0, _, _) => format!("{}{}.{:03}", prefix_str, seconds, milliseconds),
        (0, 0, _, _, _) => format!(
            "{}{}:{:02}.{:03}",
            prefix_str, minutes, seconds, milliseconds
        ),
        (0, _, _, _, _) => format!(
            "{}{}:{:02}:{:02}.{:03}",
            prefix_str, hours, minutes, seconds, milliseconds
        ),
        _ => format!(
            "{}{}:{:02}:{:02}:{:02}.{:03}",
            prefix_str, days, hours, minutes, seconds, milliseconds
        ),
    };
    String::from(&s[..(s.len() - ((3 - ms_digits) as usize))])
}

pub fn parse_color(color_hex: &str) -> Color {
    let color = u32::from_str_radix(color_hex.trim_start_matches('#'), 16).unwrap_or(0);
    Color::Rgb {
        r: ((color >> 16) & 0xFF) as u8,
        g: ((color >> 8) & 0xFF) as u8,
        b: (color & 0xFF) as u8,
    }
}

#[derive(Default, Clone)]
pub struct SegSummary {
    pub live_split: Option<Duration>,
    pub live_seg: Option<Duration>,

    // How far ahead/behind this split is compared to base
    pub live_delta: Option<Duration>,
    pub live_delta_neg: bool,

    // Time gained or lost this split relative to base
    pub gained: Option<Duration>,
    pub gained_neg: bool,

    // Convention: _s -> segment, _S -> split (accumulated segments)
    pub base_segment: Option<Duration>,
    pub base_split: Option<Duration>,

    pub gold_segment: Option<Duration>,
    pub shit_segment: Option<Duration>,
    pub is_gold_new: bool,
}

pub fn get_run_summary(timer: &TimerState) -> Vec<SegSummary> {
    let mut summary = vec![SegSummary::default(); timer.split_file.split_names.len()];

    if let Some(pb) = &timer.split_file.personal_best {
        for (i, _time) in pb.splits.iter().enumerate() {
            summary[i].base_split = baseline_split(&timer, i);
            summary[i].gold_segment = timer.split_file.golds[i].as_ref().map(|g| g.duration);
            summary[i].shit_segment = timer.split_file.shits[i].as_ref().map(|g| g.duration);
        }
        for i in 0..summary.len() {
            if i == 0 {
                summary[i].base_segment = summary[i].base_split;
            } else if let (Some(t1), Some(t3)) = 
                (summary[i].base_split, summary[i - 1].base_split) 
            {
                    summary[i].base_segment = Some(t1 - t3);
            } else {
                summary[i].base_segment = None;
            }
        }
    }

    // Calculate live split times
    for (i, split) in timer.splits.iter().enumerate() {
        summary[i].live_split = *split;
    }
    if let TimerMode::Running { start_time } = timer.mode {
        summary[timer.splits.len()].live_split = Some(start_time.elapsed());
    }

    // Calculate live segment times
    for i in 0..summary.len() {
        if i == 0 {
            summary[i].live_seg = summary[i].live_split;
        } else if let (Some(t1), Some(t2)) = (summary[i].live_split, summary[i - 1].live_split) {
            summary[i].live_seg = Some(t1 - t2);
        }
    }

    // Calculate live deltas
    #[allow(clippy::needless_range_loop)]
    for i in 0..summary.len() {
        let base = baseline_split(&timer, i);
        let live = summary[i].live_split;
        if let (Some(live), Some(base)) = (live, base) {
            // Do math in signed milliseconds because Duration is unsigned
            let ms = live.as_millis() as i64 - base.as_millis() as i64;
            summary[i].live_delta = Some(Duration::from_millis(ms.unsigned_abs()));
            summary[i].live_delta_neg = ms < 0;
        }
    }

    // Calculate live gained/lost
    for i in 0..summary.len() {
        let prev_delta = if i == 0 {
            Some(Duration::from_secs(0))
        } else {
            summary[i - 1].live_delta
        };

        if let (Some(delta1), Some(delta2)) = (summary[i].live_delta, prev_delta) {
            let prev_delta_neg = if i == 0 {
                false
            } else {
                summary[i - 1].live_delta_neg
            };

            let delta1_ms =
                delta1.as_millis() as i64 * if summary[i].live_delta_neg { -1 } else { 1 };
            let delta2_ms = delta2.as_millis() as i64 * if prev_delta_neg { -1 } else { 1 };
            let gained_ms = delta1_ms - delta2_ms;
            summary[i].gained = Some(Duration::from_millis(gained_ms.unsigned_abs()));
            summary[i].gained_neg = gained_ms < 0;
        }
    }

    // Calculate golds
    #[allow(clippy::needless_range_loop)]
    for i in 0..timer.splits.len() {
        let live_seg = summary[i].live_seg;
        let gold_seg = timer.split_file.golds[i].as_ref().map(|g| g.duration);
        let shit_seg = timer.split_file.shits[i].as_ref().map(|g| g.duration);
        match (live_seg, gold_seg) {
            (Some(live_seg), Some(gold_seg)) => {
                summary[i].gold_segment = Some(live_seg.min(gold_seg));
                summary[i].is_gold_new = live_seg < gold_seg;
            }
            (None, Some(seg)) | (Some(seg), None) => {
                summary[i].gold_segment = Some(seg);
                summary[i].is_gold_new = false;
            }
            (None, None) => {
                summary[i].gold_segment = None;
                summary[i].is_gold_new = false;
            }
        }
        match (live_seg, shit_seg) {
            (Some(live_seg), Some(shit_seg)) => {summary[i].shit_segment = Some(live_seg.max(shit_seg));}
            (None, Some(seg)) | (Some(seg), None) => {summary[i].shit_segment = Some(seg);}
            (None, None) => {summary[i].shit_segment = None;}
        }
    }
    #[allow(clippy::needless_range_loop)]
    for i in timer.splits.len()..summary.len() {
        summary[i].gold_segment = timer.split_file.golds[i].as_ref().map(|g| g.duration);
        summary[i].shit_segment = timer.split_file.shits[i].as_ref().map(|g| g.duration);
        summary[i].is_gold_new = false;
    }
    summary
}

pub fn best_possible_time(summary: &[SegSummary]) -> Option<std::time::Duration> {
    // If there is no gold for any future segment we cannot give the ideal time currently reachable.
    // In that case we return None and the UI can show "-".
    let mut total = std::time::Duration::ZERO;
    
    if summary.iter().all(|seg| seg.gold_segment.is_some()){

        for seg in summary.iter() {
            // Not-yet-completed splits.
            //    a) If we are currently on this segment (`live_seg` is Some)
            //       we take the *max* of live_seg and gold (gold may be eaten).
            //       For previous seg, the logic stays the same, if previous are worst
            //       max will do the trick and if better, they are gold, so they are 
            //       taken into account
            //    b) Otherwise we are looking at a future segment – just add gold.
            if let Some(live_seg) = seg.live_seg {
                // current segment
                let gold = seg.gold_segment.unwrap_or(std::time::Duration::ZERO);
                total += std::cmp::max(live_seg, gold);
            } else {
                // future segment
                if let Some(gold) = seg.gold_segment {
                    total += gold;
                } else {
                    return None;
                }
            }
        }
    } else {
        return None
    }
    Some(total)
}

fn baseline_split(timer: &TimerState, idx: usize) -> Option<Duration> {
    match timer.comparison {
        ComparisonMode::SumOfBest => {
            if timer.split_file.golds[idx].as_ref().is_none() {
                None
            } else {
                let mut cum = Duration::ZERO;
                for i in 0..=idx {
                    cum = cum + timer.split_file.golds[i].as_ref().unwrap().duration
                }
                Some(cum)
            }
        },
        ComparisonMode::SumOfWorst => {
            if timer.split_file.shits[idx].as_ref().is_none() {
                None
            } else {        
                let mut cum = Duration::ZERO;
                for i in 0..=idx {
                    cum = cum + timer.split_file.shits[i].as_ref().unwrap().duration
                }
                Some(cum)
            }
        },
        ComparisonMode::Average => {
            if timer.split_file.clears[idx] == 0 {
                None
            } else {        
                let mut cum = Duration::ZERO;
                for i in 0..=idx {
                    cum = cum + timer.split_file.averages[i].as_ref().unwrap().duration / timer.split_file.clears[i]
                }
                Some(cum)
            }
        },
        ComparisonMode::PersonalBest => {
            if let Some(pb) = &timer.split_file.personal_best {
                Some(pb.splits[idx].as_ref().unwrap().time)
            } else {
                None
            }
        },
        ComparisonMode::PersonalWorst => {
            if let Some(pw) = &timer.split_file.personal_worst {
                Some(pw.splits[idx].as_ref().unwrap().time)
            } else {
                None
            }
        },
    }
}
