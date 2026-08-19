use std::time::Duration;

use crossterm::style::{Attribute, Color};
use crate::timer::ComparisonMode;

use crate::{
    bigtext::get_big_text,
    rotty::{Block, Image, TextAlign},
    settings::Theme,
    timer_state::{TimerMode, TimerState},
    utils::{format_duration, get_run_summary, best_possible_time, parse_color, Prefix, SegSummary, Sign, },
};

#[derive(Clone, Copy)]
struct ColumnWidths {
    name: u16,    // split name
    clears: u16,  // number of clears (the new column)
    delta: u16,   // +/- delta
    seg: u16,     // segment time
    split: u16,   // split time
}

static TIMER_WIDTH: u16 = 48;
const DEFAULT_COLS: ColumnWidths = ColumnWidths {
    name: 17,
    clears: 4,
    delta: 5,
    seg: 11,
    split: 11,
};

fn header_cell(text: &str, width: u16, theme: &Theme) -> Block {
    Image::new(text, width, TextAlign::Right)
        .fg_color(parse_color(theme.label_text))
        .build()
}

pub fn render_view(timer: &TimerState, theme: &Theme) -> Block {
    let summary = get_run_summary(timer);

    let elapsed = match timer.mode {
        TimerMode::Initial => Duration::from_secs(0),
        TimerMode::Running { start_time } => start_time.elapsed(),
        TimerMode::Paused { elapsed_at_pause } => elapsed_at_pause,
        TimerMode::Finished { start_time: _ } => timer.splits.last().unwrap().unwrap(),
    };
    let precision = precision_for_mode(&timer.mode);

    let mode_label = match timer.comparison {
        ComparisonMode::PersonalBest => "Comparison: Personal Best",
        ComparisonMode::PersonalWorst => "Comparison: Personal Worst",
        ComparisonMode::SumOfBest => "Comparison: Sum of Best",
        ComparisonMode::SumOfWorst => "Comparison: Sum of Worst",
        ComparisonMode::Average => "Comparison: Average",
    };

    let title = &timer.split_file.title;
    let category = &timer.split_file.category;
    let title_block = Image::new(title, TIMER_WIDTH, TextAlign::Center)
        .attr(Attribute::Bold)
        .build();
    let category_block = Image::new(category, TIMER_WIDTH, TextAlign::Center)
        .attr(Attribute::Bold)
        .build();

    let mode_block = Image::new(mode_label, TIMER_WIDTH, TextAlign::Right)
        .fg_color(parse_color(theme.label_text))
        .build();

    let elap = match timer.mode {
        TimerMode::Running {start_time} => start_time.elapsed(),
        TimerMode::Paused {elapsed_at_pause} => elapsed_at_pause,
        TimerMode::Finished {..} => {
            let splits = &timer.splits;
            splits.last().unwrap().unwrap()
        },
        _ => Duration::ZERO,
    };

    let runtime_text = format!(
        "Total runtime: {}",
        format_duration(
            timer.split_file.total_runtime + elap, 
            -1, 
            Sign::Positive, 
            Prefix::NoneOrMinus)
    );
    let runtime_block = Image::new(&runtime_text, TIMER_WIDTH / 2, TextAlign::Left).build();

    let attempts_text = format!(
        "{}/{}",
        timer.split_file.completed, timer.split_file.attempts
    );
    let attempts_block = Image::new(&attempts_text, TIMER_WIDTH / 2, TextAlign::Right).build();
    let th_block = runtime_block.horiz(attempts_block);

    let spacer_block = Image::new(
        &" ".repeat(TIMER_WIDTH as usize),
        TIMER_WIDTH,
        TextAlign::Left,
    )
    .build();

    let cols = DEFAULT_COLS; 
    let headers = [
        header_cell("", cols.name, theme),          
        header_cell("Clr", cols.clears, theme),    
        header_cell("Δ", cols.delta, theme),       
        header_cell("Seg", cols.seg, theme),       
        header_cell("Split", cols.split, theme),   
    ];
    let header_row = Block::hcat(headers);

    let line_sep = Image::new(
        &"─".repeat(TIMER_WIDTH as usize),
        TIMER_WIDTH,
        TextAlign::Left,
    )
    .fg_color(parse_color(theme.label_text))
    .build();

    let split_rows: Vec<Block> = (0..timer.split_file.split_names.len())
        .map(|i| get_split_row(timer, i as u32, theme, &summary, precision))
        .collect();

    let mut sections = vec![
        title_block,
        category_block,
        th_block,
        mode_block,
        //spacer_block.clone(),
        header_row,
        line_sep.clone(),
    ];
    sections.extend(split_rows);
    sections.extend([
        line_sep,
        spacer_block.clone(),
        get_big_timer(timer, theme, &summary, elapsed, precision),
        spacer_block,
        get_prev_segment_block(timer, theme, &summary, precision),
        get_sum_of_best_block(&summary),
        get_best_possible_block(&summary),
    ]);
    Block::vcat(sections)
}

fn get_big_timer(
    timer: &TimerState,
    theme: &Theme,
    summary: &[SegSummary],
    elapsed: Duration,
    precision: i32
) -> Block {
    let color = match timer.mode {
        TimerMode::Initial => parse_color(theme.ahead_gain),
        TimerMode::Running { .. } => get_delta_color(timer.splits.len() as u32, theme, summary),
        TimerMode::Paused { .. } => parse_color(theme.paused_text),
        TimerMode::Finished { .. } => {
            if summary[summary.len() - 1].live_delta_neg {
                get_rainbow_color(timer)
            } else {
                parse_color(theme.behind_lose)
            }
        }
    };

    get_big_text(&format_duration(
        elapsed,
        precision,
        Sign::Positive,
        Prefix::NoneOrMinus,
    ))
    .left_pad(TIMER_WIDTH)
    .fg_color(color)
}

fn get_split_row(timer: &TimerState, idx: u32, theme: &Theme, summary: &[SegSummary], precision: i32) -> Block {
    let split_name = &timer.split_file.split_names[idx as usize];
    let name_col = Image::new(split_name, DEFAULT_COLS.name, TextAlign::Left).build();
    let clears = timer
        .split_file
        .clears
        .get(idx as usize)
        .cloned()
        .unwrap_or(0);
    let clears_text = clears.to_string();
    let clears_col = Image::new(&clears_text, DEFAULT_COLS.clears, TextAlign::Right).build();

    // Build segment text
    let seg_dur = if (idx as usize) < timer.splits.len() {
        summary[idx as usize].live_seg
    } else {
        summary[idx as usize].base_segment
    };
    let seg_text = match seg_dur {
        Some(seg_dur) => format_duration(seg_dur, precision, Sign::Positive, Prefix::NoneOrMinus),
        None => "-".to_string(),
    };

    // Build split text
    let split_dur = if (idx as usize) < timer.splits.len() {
        summary[idx as usize].live_split
    } else {
        summary[idx as usize].base_split
    };
    let split_text = match split_dur {
        Some(split_dur) => format_duration(split_dur, precision, Sign::Positive, Prefix::NoneOrMinus),
        None => "-".to_string(),
    };

    let seg_col = Image::new(&seg_text, DEFAULT_COLS.seg, TextAlign::Right).build();
    let split_col = Image::new(&split_text, DEFAULT_COLS.split, TextAlign::Right).build();
    let delta_col = get_delta_block(timer, idx, theme, summary);

    let running = matches!(timer.mode, TimerMode::Running { start_time: _ });
    let mut bg_image = Image::new(
        &" ".repeat(TIMER_WIDTH as usize),
        TIMER_WIDTH,
        TextAlign::Left,
    );
    if running && idx as usize == timer.splits.len() {
        bg_image = bg_image.bg_color(parse_color(theme.highlight));
    }
    let bg = bg_image.build();

    bg.stack(Block::hcat([name_col, clears_col, delta_col, seg_col, split_col]))
}

fn get_delta_color(idx: u32, theme: &Theme, summary: &[SegSummary]) -> Color {
    if summary[idx as usize].live_delta.is_some() {
        let delta_neg = summary[idx as usize].live_delta_neg;
        let gain_neg = if summary[idx as usize].gained.is_some() {
            summary[idx as usize].gained_neg
        } else {
            delta_neg
        };

        let color_str = match (delta_neg, gain_neg) {
            (true, true) => theme.ahead_gain,
            (true, false) => theme.ahead_lose,
            (false, true) => theme.behind_gain,
            (false, false) => theme.behind_lose,
        };
        parse_color(color_str)
    } else {
        parse_color(theme.ahead_gain)
    }
}

fn get_delta_block(timer: &TimerState, idx: u32, theme: &Theme, summary: &[SegSummary]) -> Block {
    if let Some(delta) = summary[idx as usize].live_delta {
        // If delta is for the upcoming split:
        // - Hide until segment time exceeds gold, if both exist
        // - Hide till split time exceeds PB split time, if both exist
        // - Else hide indefinitely?
        let show = if let (Some(seg), Some(gold)) =
            (summary[idx as usize].live_seg, summary[idx as usize].gold_segment)
        {
            seg >= gold
        } else if let (Some(live_split), Some(base_split)) = (
            summary[idx as usize].live_split,
            summary[idx as usize].base_split,
        ) {
            live_split >= base_split
        } else {
            true
        };

        if show {
            let dur_str = format_duration(
                delta,
                1,
                (!summary[idx as usize].live_delta_neg).into(),
                Prefix::PlusOrMinus,
            );
            let color = if summary[idx as usize].is_gold_new {
                get_rainbow_color(timer)
            } else {
                get_delta_color(idx, theme, summary)
            };
            Image::new(&dur_str, DEFAULT_COLS.delta, TextAlign::Right)
                .fg_color(color)
                .build()
        } else {
            Image::new(" ", DEFAULT_COLS.delta, TextAlign::Left).build()
        }
    } else {
        Image::new("-", DEFAULT_COLS.delta, TextAlign::Right).build()
    }
}

fn get_prev_segment_block(timer: &TimerState, theme: &Theme, summary: &[SegSummary], precision: i32) -> Block {
    let gained_dur = if timer.splits.is_empty() {
        None
    } else {
        summary[timer.splits.len() - 1].gained
    };

    let color;
    let s;
    if let Some(gained_dur) = gained_dur {
        let neg = summary[timer.splits.len() - 1].gained_neg;
        s = format_duration(gained_dur, precision, (!neg).into(), Prefix::PlusOrMinus);
        color = if neg {
            parse_color(theme.ahead_gain)
        } else {
            parse_color(theme.behind_lose)
        };
    } else {
        color = parse_color(theme.normal_text);
        s = "-".to_string();
    };

    let label_col = Image::new("Previous Segment", TIMER_WIDTH / 2, TextAlign::Left).build();
    let prev_seg_col = Image::new(&s, TIMER_WIDTH - TIMER_WIDTH / 2, TextAlign::Right)
        .fg_color(color)
        .build();
    label_col.horiz(prev_seg_col)
}

fn get_sum_of_best_block(summary: &[SegSummary]) -> Block {
    let sob_text = if summary.iter().all(|seg| seg.gold_segment.is_some()) {
        let sob = summary.iter().map(|seg| seg.gold_segment.as_ref().unwrap()).sum();
        format_duration(sob, 3, Sign::Positive, Prefix::NoneOrMinus)
    } else {
        "-".to_string()
    };

    let label_col = Image::new("Sum of Best Segments", TIMER_WIDTH / 2, TextAlign::Left).build();
    let sob_col = Image::new(&sob_text, TIMER_WIDTH - TIMER_WIDTH / 2, TextAlign::Right).build();
    label_col.horiz(sob_col)
}

fn get_best_possible_block(summary: &[SegSummary]) -> Block {
    let best_opt = best_possible_time(summary);

    let text = if let Some(best) = best_opt {
        format_duration(best, 3, Sign::Positive, Prefix::NoneOrMinus)
    } else {
        "-".to_string()
    };

    let label = Image::new("Best possible time", TIMER_WIDTH / 2, TextAlign::Left).build();
    let value = Image::new(&text, TIMER_WIDTH - TIMER_WIDTH / 2, TextAlign::Right).build();

    label.horiz(value)
}

fn hsl_to_color(h: f64, s: f64, l: f64) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r, g, b) = match (h * 6.0).floor() as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    Color::Rgb {
        r: ((r + m) * 255.0) as u8,
        g: ((g + m) * 255.0) as u8,
        b: ((b + m) * 255.0) as u8,
    }
}

fn get_rainbow_color(timer: &TimerState) -> Color {
    let loop_duration = Duration::from_secs(3);
    let t = timer
        .anim_ref_time
        .elapsed()
        .div_duration_f64(loop_duration)
        % 1.0;
    hsl_to_color(t, 1.0, 0.6)
}

// precision function: allows to have different accuracy at the end of the run.
fn precision_for_mode(mode: &TimerMode) -> i32 {
    match mode {
        TimerMode::Initial | TimerMode::Finished { .. } | TimerMode::Paused {..} => 3,
        _ => 1, 
    }
}

