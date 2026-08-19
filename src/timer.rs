use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::style::Color;
use device_query::{DeviceQuery, DeviceState, Keycode};

use crate::settings::{self, Action, Settings};
use crate::split_file::{write_split_file, Segment, ActualRun, Split};
use crate::timer_state::{TimerMode, TimerState};
use crate::utils::{get_run_summary, parse_color};
use crate::{rotty::Renderer, split_file::read_split_file, view};

pub struct Timer {
    device_state: DeviceState,
    renderer: Renderer,
    timer_state: TimerState,
    settings: Settings,
    prev_keys: HashSet<Keycode>,
}

// Introduction of the possible comparison modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonMode {
    PersonalBest,
    PersonalWorst,
    SumOfBest,
    SumOfWorst,
    Average,
}

impl Timer {
    pub fn new(splits_file: &Path, config_path: &Path) -> anyhow::Result<Self> {
        let split_file = read_split_file(splits_file).context("Failed to read splits file")?;

        let settings = if config_path.exists() {
            settings::read_settings_file(config_path).context("Failed to read settings file")?
        } else {
            settings::DEFAULT_SETTINGS.clone()
        };

        Ok(Self {
            device_state: DeviceState::new(),
            renderer: Renderer::new(),
            timer_state: TimerState {
                split_file,
                splits: Vec::new(),
                mode: TimerMode::Initial,
                anim_ref_time: Instant::now(),
                comparison: ComparisonMode::PersonalBest,
            },
            settings,
            prev_keys: HashSet::new(),
        })
    }

    pub fn update(&mut self, _: f32) -> anyhow::Result<bool> {
        // read inputs performed in the terminal only
        if read_terminal_key_chars()?.contains(&'q') {
            self.reset_to_initial_mode()?;
            return Ok(false);
        }

        // gather keys that are pressed using device_query
        let global_keys: HashSet<Keycode> = self.device_state.get_keys().into_iter().collect();
        // filter keys with valid actions only
        let actions: Vec<Action> = global_keys
            .iter()
            .filter(|key| !self.prev_keys.contains(key))
            .flat_map(|key| self.settings.global_hotkeys.get(key).copied())
            .collect();
        // provide internally the set of action to perform
        self.prev_keys = global_keys;

        // apply actions
        for action in actions {
            self.apply_action(action)?;
        }

        // once state is updated make the visuals
        let bg_color = if self.settings.draw_background {
            parse_color(self.settings.theme.bg)
        } else {
            Color::Reset
        };
        self.renderer
            .set_default_colors(parse_color(self.settings.theme.normal_text), bg_color);

        let block = view::render_view(&self.timer_state, self.settings.theme);
        self.renderer.render(&block)?;
        Ok(true)
    }
    // When an action is catch, match the different outcomes, depending on the current status
    pub fn apply_action(&mut self, action: Action) -> anyhow::Result<()> {
        match self.timer_state.mode {
            #[allow(clippy::single_match)]
            // Before starting the run, detect if it started
            TimerMode::Initial => match action {
                Action::Split => {
                    self.timer_state.mode = TimerMode::Running {
                        start_time: Instant::now(),
                    };
                }
                Action::ComparisonBetter => {self.comparison_better();}
                Action::ComparisonWorse => {self.comparison_worse();}
                _ => {}
            },
            // When paused, detects unpause, resets, and deletes
            TimerMode::Paused { elapsed_at_pause } => match action {
                Action::Pause => {
                    let new_start_time = Instant::now() - elapsed_at_pause;
                    self.timer_state.mode = TimerMode::Running {
                        start_time: new_start_time,
                    };
                }
                Action::ResetAndSave => {
                    self.timer_state.split_file.attempts += 1;
                    self.save_time_segment()?;
                    self.reset_to_initial_mode()?;
                }
                Action::ResetAndDelete => {
                    self.reset_to_initial_mode()?;
                }
                Action::ComparisonBetter => {self.comparison_better();}
                Action::ComparisonWorse => {self.comparison_worse();}
                _ => {}
            },
            // When running, detects pause, split, unsplit, resets and deletes
            TimerMode::Running { start_time } => match action {
                Action::Split => {
                    let elapsed = start_time.elapsed();
                    self.timer_state.splits.push(Some(elapsed));
                    if self.timer_state.splits.len()
                        == self.timer_state.split_file.split_names.len()
                    {
                        self.timer_state.mode = TimerMode::Finished { start_time };
                    }
                }
                Action::UndoSplit => {
                    if self.timer_state.splits.is_empty() {
                        self.reset_to_initial_mode()?;
                    } else {
                        self.timer_state.splits.pop();
                    }
                }
                Action::DeleteSplit => {
                    if !self.timer_state.splits.is_empty() {
                        let len = self.timer_state.splits.len();
                        self.timer_state.splits[len - 1] = None;
                    }
                }
                Action::ResetAndSave => {
                    self.timer_state.split_file.attempts += 1;
                    self.save_time_segment()?; // Also saves attempts
                    self.reset_to_initial_mode()?;
                }
                Action::ResetAndDelete => {
                    self.reset_to_initial_mode()?;
                }
                Action::Pause => {
                    self.timer_state.mode = TimerMode::Paused {
                        elapsed_at_pause: start_time.elapsed(),
                    };
                }
                Action::ComparisonBetter => {self.comparison_better();}
                Action::ComparisonWorse => {self.comparison_worse();}
            },
            // When finished, detects unsplit, save and deletes
            TimerMode::Finished { start_time } => match action {
                Action::UndoSplit => {
                    self.timer_state.splits.pop();
                    self.timer_state.mode = TimerMode::Running { start_time };
                }
                Action::ResetAndSave => {
                    self.timer_state.split_file.attempts += 1;
                    self.timer_state.split_file.completed += 1;
                    self.save_time_segment()?; // Also saves attempts/completed
                    self.save_personal_best()?;
                    self.reset_to_initial_mode()?;
                }
                Action::ResetAndDelete => {
                    self.reset_to_initial_mode()?;
                }
                Action::ComparisonBetter => {self.comparison_better();}
                Action::ComparisonWorse => {self.comparison_worse();}
                _ => {}
            },
        }
        Ok(())
    }
   
    fn comparison_better(&mut self) {
        self.timer_state.comparison = match self.timer_state.comparison { 
            ComparisonMode::SumOfBest => ComparisonMode::SumOfBest,
            ComparisonMode::PersonalBest => ComparisonMode::SumOfBest,
            ComparisonMode::Average => ComparisonMode::PersonalBest,
            ComparisonMode::PersonalWorst => ComparisonMode::Average,
            ComparisonMode::SumOfWorst => ComparisonMode::PersonalWorst,
        };
    }
    
    fn comparison_worse(&mut self) {
        self.timer_state.comparison = match self.timer_state.comparison { 
            ComparisonMode::SumOfBest => ComparisonMode::PersonalBest,
            ComparisonMode::PersonalBest => ComparisonMode::Average,
            ComparisonMode::Average => ComparisonMode::PersonalWorst,
            ComparisonMode::PersonalWorst => ComparisonMode::SumOfWorst,
            ComparisonMode::SumOfWorst => ComparisonMode::SumOfWorst,
        };
    }

    // private functions called for the primary actions: reset, save the state
    fn reset_to_initial_mode(&mut self) -> anyhow::Result<()> {
        let elap = match self.timer_state.mode {
            TimerMode::Running {start_time} => start_time.elapsed(),
            TimerMode::Paused {elapsed_at_pause} => elapsed_at_pause,
            TimerMode::Finished {..} => {
                let splits = &self.timer_state.splits;
                splits.last().unwrap().unwrap()
            },
            _ => Duration::ZERO,
        };
        self.timer_state.split_file.total_runtime = self.timer_state.split_file.total_runtime + elap;

        write_split_file(&self.timer_state.split_file)?;
        
        self.timer_state.mode = TimerMode::Initial;
        self.timer_state.splits.clear();
        Ok(())
    }

    fn save_time_segment(&mut self) -> anyhow::Result<()> {
        let n = self.timer_state.splits.len();
        let run_summary = get_run_summary(&self.timer_state);

        for i in 0..n {
            let file_golds = &mut self.timer_state.split_file.golds;
            file_golds[i] = run_summary[i].gold_segment.as_ref().map(|&g| Segment { duration: g });
            let file_aves = &mut self.timer_state.split_file.averages;
            file_aves[i] = Some(Segment {
                duration: file_aves[i].as_ref().unwrap().duration +
                    run_summary[i].live_seg.as_ref().map(|&g| Segment { duration: g }).unwrap().duration
            });
            let file_shits = &mut self.timer_state.split_file.shits;
            file_shits[i] = run_summary[i].shit_segment.as_ref().map(|&g| Segment { duration: g });
            let file_clears = &mut self.timer_state.split_file.clears;
            file_clears[i] += 1;
        }
        Ok(())
    }

    fn save_personal_best(&mut self) -> anyhow::Result<()> {
        let splits = &self.timer_state.splits;
        let curr_time = splits.last().unwrap().unwrap();

        let should_save1 = match &self.timer_state.split_file.personal_best {
            Some(pb) => {
                let pb_time = pb.splits.last().unwrap().as_ref().unwrap().time;
                curr_time < pb_time
            }
            None => true, 
        };
        let should_save2 = match &self.timer_state.split_file.personal_worst {
            Some(pw) => {
                let pw_time = pw.splits.last().unwrap().as_ref().unwrap().time;
                curr_time > pw_time
            }
            None => true, 
        };

        if should_save1 {
            let pb = ActualRun {
                attempt: self.timer_state.split_file.attempts,
                splits: splits.iter().map(|s| s.map(|dur| Split { time: dur })).collect(),
            };
            self.timer_state.split_file.personal_best = Some(pb);
        }
        if should_save2 {
            let pw = ActualRun {
                attempt: self.timer_state.split_file.attempts,
                splits: splits.iter().map(|s| s.map(|dur| Split { time: dur })).collect(),
            };
            self.timer_state.split_file.personal_worst = Some(pw);
        }
        Ok(())
    }
}

// reads keys that are pressed within the terminal to look later if it is "q" to end the program
fn read_terminal_key_chars() -> anyhow::Result<Vec<char>> {
    let mut input_chars = Vec::new();

    while event::poll(Duration::from_secs(0))? {
        if let Event::Key(KeyEvent {
            code: KeyCode::Char(c),
            kind: KeyEventKind::Press,
            modifiers: _,
            state: _,
        }) = event::read()?
        {
            input_chars.push(c);
        }
    }

    Ok(input_chars)
}

