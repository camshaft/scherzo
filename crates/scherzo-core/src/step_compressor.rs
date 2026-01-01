use std::collections::VecDeque;
use thiserror::Error;

const QUEUE_START_SIZE: usize = 1024;
const CLOCK_DIFF_MAX: u64 = 3 << 28;
const QUADRATIC_DEV: i64 = 11; // (6 + 4*sqrt(2)) ~= 11.65, but 11 is used upstream.
const SDS_FILTER_TIME: f64 = 0.000_750;

#[derive(Debug, Error)]
pub enum StepCompressError {
    #[error("invalid sequence i={interval} c={count} a={add}")]
    InvalidSequence { interval: u32, count: u16, add: i16 },
    #[error(
        "point {index} out of range: {value} not in {min}:{max} for i={interval} c={count} a={add}"
    )]
    PointOutOfRange {
        index: u16,
        value: i64,
        min: i64,
        max: i64,
        interval: u32,
        count: u16,
        add: i16,
    },
    #[error("interval overflow at point {index} for i={interval} c={count} a={add}")]
    IntervalOverflow {
        index: u16,
        interval: u32,
        count: u16,
        add: i16,
    },
}

pub type Result<T> = std::result::Result<T, StepCompressError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueStep {
    pub oid: u32,
    pub first_clock: u64,
    pub last_clock: u64,
    pub interval: u32,
    pub count: u16,
    pub add: i16,
    pub req_clock: u64,
    pub min_clock: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetNextStepDir {
    pub oid: u32,
    pub dir: bool,
    pub req_clock: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    QueueStep(QueueStep),
    SetNextStepDir(SetNextStepDir),
}

pub trait CommandSink {
    fn push(&mut self, command: Command);
}

#[derive(Default, Debug)]
pub struct RecordingSink {
    pub commands: Vec<Command>,
}

impl CommandSink for RecordingSink {
    fn push(&mut self, command: Command) {
        self.commands.push(command);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullHistoryStep {
    pub first_clock: u64,
    pub last_clock: u64,
    pub start_position: i64,
    pub step_count: i32,
    pub interval: u32,
    pub add: i16,
}

#[derive(Clone, Debug)]
struct HistoryEntry {
    first_clock: u64,
    last_clock: u64,
    start_position: i64,
    step_count: i32,
    interval: u32,
    add: i16,
}

#[derive(Copy, Clone, Debug)]
struct StepMove {
    interval: u32,
    count: u16,
    add: i16,
}

#[derive(Copy, Clone, Debug)]
struct Points {
    minp: i64,
    maxp: i64,
}

fn idiv_up(n: i64, d: i64) -> i64 {
    if n >= 0 { (n + d - 1) / d } else { n / d }
}

fn idiv_down(n: i64, d: i64) -> i64 {
    if n >= 0 { n / d } else { (n - d + 1) / d }
}

pub struct StepCompressor<S: CommandSink> {
    oid: u32,
    max_error: u32,
    mcu_time_offset: f64,
    mcu_freq: f64,
    last_step_print_time: f64,
    last_step_clock: u64,
    // direction tracking
    sdir: i32,
    invert_sdir: bool,
    next_step_clock: Option<u64>,
    next_step_dir: i32,
    // buffering
    queue: Vec<u64>,
    queue_pos: usize,
    // history
    last_position: i64,
    history: VecDeque<HistoryEntry>,
    // output
    sink: S,
}

impl<S: CommandSink> StepCompressor<S> {
    pub fn new(oid: u32, max_error: u32, sink: S) -> Self {
        Self {
            oid,
            max_error,
            mcu_time_offset: 0.0,
            mcu_freq: 1.0,
            last_step_print_time: -0.5,
            last_step_clock: 0,
            sdir: -1,
            invert_sdir: false,
            next_step_clock: None,
            next_step_dir: 0,
            queue: Vec::with_capacity(QUEUE_START_SIZE),
            queue_pos: 0,
            last_position: 0,
            history: VecDeque::new(),
            sink,
        }
    }

    pub fn set_time(&mut self, time_offset: f64, mcu_freq: f64) {
        self.mcu_time_offset = time_offset;
        self.mcu_freq = mcu_freq;
        self.calc_last_step_print_time();
    }

    pub fn set_invert_sdir(&mut self, invert: bool) {
        if self.invert_sdir != invert {
            self.invert_sdir = invert;
            if self.sdir >= 0 {
                self.sdir ^= 1;
            }
        }
    }

    pub fn get_last_dir(&self) -> bool {
        // Returns the last commanded step direction
        // If no step has been commanded yet, returns false (direction 0)
        if self.sdir < 0 { false } else { self.sdir != 0 }
    }

    pub fn set_last_position(&mut self, clock: u64, last_position: i64) -> Result<()> {
        self.flush(u64::MAX)?;
        self.last_position = last_position;
        self.history.push_front(HistoryEntry {
            first_clock: clock,
            last_clock: clock,
            start_position: last_position,
            step_count: 0,
            interval: 0,
            add: 0,
        });
        Ok(())
    }

    pub fn reset(&mut self, last_step_clock: u64) -> Result<()> {
        self.flush(u64::MAX)?;
        self.last_step_clock = last_step_clock;
        self.sdir = -1;
        self.calc_last_step_print_time();
        Ok(())
    }

    pub fn append(&mut self, sdir: i32, print_time: f64, step_time: f64) -> Result<()> {
        // Calculate step clock
        let offset = print_time - self.last_step_print_time;
        let rel_sc = (step_time + offset) * self.mcu_freq;
        let step_clock = self.last_step_clock + rel_sc as u64;

        if let Some(prev_clock) = self.next_step_clock {
            if sdir != self.next_step_dir {
                let diff = step_clock as i64 - prev_clock as i64;
                if (diff as f64) < SDS_FILTER_TIME * self.mcu_freq {
                    // rollback last step to avoid rapid step+dir+step
                    self.next_step_clock = None;
                    self.next_step_dir = sdir;
                    return Ok(());
                }
            }
            self.queue_append()?;
        }

        self.next_step_clock = Some(step_clock);
        self.next_step_dir = sdir;
        Ok(())
    }

    pub fn commit(&mut self) -> Result<()> {
        if self.next_step_clock.is_some() {
            self.queue_append()?;
        }
        Ok(())
    }

    pub fn flush(&mut self, move_clock: u64) -> Result<()> {
        if let Some(next_clock) = self.next_step_clock
            && move_clock >= next_clock
        {
            self.queue_append()?;
        }
        self.queue_flush(move_clock)
    }

    pub fn find_past_position(&self, clock: u64) -> i64 {
        let mut last_position = self.last_position;
        for entry in &self.history {
            if clock < entry.first_clock {
                last_position = entry.start_position;
                continue;
            }
            if clock >= entry.last_clock {
                return entry.start_position + entry.step_count as i64;
            }

            let interval = entry.interval as i64;
            let add = entry.add as i64;
            let ticks = (clock as i64 - entry.first_clock as i64) + interval;
            let offset = if add == 0 {
                ticks as f64 / interval as f64
            } else {
                // quadratic solve
                let a = 0.5_f64 * add as f64;
                let b = interval as f64 - 0.5_f64 * add as f64;
                let c = -ticks as f64;
                ((b * b - 4.0 * a * c).sqrt() - b) / (2.0 * a)
            } as i64;

            return if entry.step_count < 0 {
                entry.start_position - offset
            } else {
                entry.start_position + offset
            };
        }
        last_position
    }

    pub fn extract_old(
        &self,
        max: usize,
        start_clock: u64,
        end_clock: u64,
    ) -> Vec<PullHistoryStep> {
        let mut res = Vec::new();
        for entry in &self.history {
            if start_clock >= entry.last_clock || res.len() >= max {
                break;
            }
            if end_clock <= entry.first_clock {
                continue;
            }
            res.push(PullHistoryStep {
                first_clock: entry.first_clock,
                last_clock: entry.last_clock,
                start_position: entry.start_position,
                step_count: entry.step_count,
                interval: entry.interval,
                add: entry.add,
            });
        }
        res
    }

    pub fn expire_history(&mut self, end_clock: u64) {
        while let Some(back) = self.history.back() {
            if back.last_clock > end_clock {
                break;
            }
            self.history.pop_back();
        }
    }

    pub fn last_position(&self) -> i64 {
        self.last_position
    }

    pub fn last_step_clock(&self) -> u64 {
        self.last_step_clock
    }

    pub fn into_sink(self) -> S {
        self.sink
    }

    // --- internals ---
    fn calc_last_step_print_time(&mut self) {
        let lsc = self.last_step_clock as f64;
        self.last_step_print_time = self.mcu_time_offset + (lsc - 0.5) / self.mcu_freq;
    }

    fn minmax_point(&self, idx: usize) -> Points {
        let lsc = self.last_step_clock as i64;
        let point = self.queue[idx] as i64 - lsc;
        let prevpoint = if idx > self.queue_pos {
            self.queue[idx - 1] as i64 - lsc
        } else {
            0
        };
        let mut max_error = (point - prevpoint) / 2;
        if max_error > self.max_error as i64 {
            max_error = self.max_error as i64;
        }
        Points {
            minp: point - max_error,
            maxp: point,
        }
    }

    fn compress_bisect_add(&self) -> StepMove {
        let queue_len = self.queue.len();
        let qlast = (self.queue_pos + 65_535).min(queue_len);
        let point = self.minmax_point(self.queue_pos);
        let mut outer_mininterval = point.minp;
        let mut outer_maxinterval = point.maxp;
        let mut add: i64 = 0;
        let mut minadd: i64 = -0x8000;
        let mut maxadd: i64 = 0x7fff;
        let mut bestinterval: i64 = 0;
        let mut bestcount: i64 = 1;
        let mut bestadd: i64 = 1;
        let mut bestreach: i64 = i64::MIN;
        let mut zerointerval: i64 = 0;
        let mut zerocount: i64 = 0;

        loop {
            let mut nextpoint;
            let mut nextmininterval = outer_mininterval;
            let mut nextmaxinterval = outer_maxinterval;
            let mut interval = nextmaxinterval;
            let mut nextcount: i64 = 1;
            loop {
                nextcount += 1;
                if self.queue_pos + (nextcount as usize) > qlast {
                    let count = nextcount - 1;
                    return StepMove {
                        interval: interval as u32,
                        count: count as u16,
                        add: add as i16,
                    };
                }
                nextpoint = self.minmax_point(self.queue_pos + nextcount as usize - 1);
                let nextaddfactor = nextcount * (nextcount - 1) / 2;
                let c = add * nextaddfactor;
                if nextmininterval * nextcount < nextpoint.minp - c {
                    nextmininterval = idiv_up(nextpoint.minp - c, nextcount);
                }
                if nextmaxinterval * nextcount > nextpoint.maxp - c {
                    nextmaxinterval = idiv_down(nextpoint.maxp - c, nextcount);
                }
                if nextmininterval > nextmaxinterval {
                    break;
                }
                interval = nextmaxinterval;
            }

            let count = nextcount - 1;
            let addfactor = count * (count - 1) / 2;
            let reach = add * addfactor + interval * count;
            if reach > bestreach || (reach == bestreach && interval > bestinterval) {
                bestinterval = interval;
                bestcount = count;
                bestadd = add;
                bestreach = reach;
                if add == 0 {
                    zerointerval = interval;
                    zerocount = count;
                }
                if count > 0x200 {
                    break;
                }
            }

            let nextaddfactor = nextcount * (nextcount - 1) / 2;
            let nextreach = add * nextaddfactor + interval * nextcount;
            if nextreach < nextpoint.minp {
                minadd = add + 1;
                outer_maxinterval = nextmaxinterval;
            } else {
                maxadd = add - 1;
                outer_mininterval = nextmininterval;
            }

            if count > 1 {
                let errdelta = self.max_error as i64 * QUADRATIC_DEV / (count * count);
                if minadd < add - errdelta {
                    minadd = add - errdelta;
                }
                if maxadd > add + errdelta {
                    maxadd = add + errdelta;
                }
            }

            let c = outer_maxinterval * nextcount;
            if minadd * nextaddfactor < nextpoint.minp - c {
                minadd = idiv_up(nextpoint.minp - c, nextaddfactor);
            }
            let c2 = outer_mininterval * nextcount;
            if maxadd * nextaddfactor > nextpoint.maxp - c2 {
                maxadd = idiv_down(nextpoint.maxp - c2, nextaddfactor);
            }

            if minadd > maxadd {
                break;
            }
            add = maxadd - (maxadd - minadd) / 4;
        }

        if zerocount + zerocount / 16 >= bestcount {
            return StepMove {
                interval: zerointerval as u32,
                count: zerocount as u16,
                add: 0,
            };
        }

        StepMove {
            interval: bestinterval as u32,
            count: bestcount as u16,
            add: bestadd as i16,
        }
    }

    fn check_line(&self, mv: StepMove) -> Result<()> {
        if mv.count == 0
            || (mv.interval == 0 && mv.add == 0 && mv.count > 1)
            || mv.interval >= 0x8000_0000
        {
            return Err(StepCompressError::InvalidSequence {
                interval: mv.interval,
                count: mv.count,
                add: mv.add,
            });
        }

        let mut interval = mv.interval as i64;
        let mut p: i64 = 0;
        for i in 0..mv.count {
            let point = self.minmax_point(self.queue_pos + i as usize);
            p += interval;
            if p < point.minp || p > point.maxp {
                return Err(StepCompressError::PointOutOfRange {
                    index: i + 1,
                    value: p,
                    min: point.minp,
                    max: point.maxp,
                    interval: mv.interval,
                    count: mv.count,
                    add: mv.add,
                });
            }
            if interval >= 0x8000_0000 {
                return Err(StepCompressError::IntervalOverflow {
                    index: i + 1,
                    interval: mv.interval,
                    count: mv.count,
                    add: mv.add,
                });
            }
            interval += mv.add as i64;
        }
        Ok(())
    }

    fn add_move(&mut self, first_clock: u64, mv: &StepMove) {
        let addfactor = mv.count as u64 * (mv.count as u64 - 1) / 2;
        let ticks = mv.add as i64 * addfactor as i64 + mv.interval as i64 * (mv.count as i64 - 1);
        let last_clock = first_clock + ticks as u64;

        let mut req_clock = self.last_step_clock;
        let min_clock = req_clock;
        if mv.count == 1 && first_clock >= self.last_step_clock + CLOCK_DIFF_MAX {
            req_clock = first_clock;
        }

        self.sink.push(Command::QueueStep(QueueStep {
            oid: self.oid,
            first_clock,
            last_clock,
            interval: mv.interval,
            count: mv.count,
            add: mv.add,
            req_clock,
            min_clock,
        }));
        self.last_step_clock = last_clock;

        let step_count = if self.sdir != 0 {
            mv.count as i32
        } else {
            -(mv.count as i32)
        };
        let entry = HistoryEntry {
            first_clock,
            last_clock,
            start_position: self.last_position,
            step_count,
            interval: mv.interval,
            add: mv.add,
        };
        self.last_position += step_count as i64;
        self.history.push_front(entry);
    }

    fn queue_flush(&mut self, move_clock: u64) -> Result<()> {
        if self.queue_pos >= self.queue.len() {
            return Ok(());
        }

        while self.last_step_clock < move_clock {
            let mv = self.compress_bisect_add();
            self.check_line(mv)?;
            let first_clock = self.last_step_clock + mv.interval as u64;
            self.add_move(first_clock, &mv);

            let advance = mv.count as usize;
            if self.queue_pos + advance >= self.queue.len() {
                self.queue.clear();
                self.queue_pos = 0;
                break;
            }
            self.queue_pos += advance;
        }
        self.calc_last_step_print_time();
        if self.queue_pos > 0 && self.queue_pos * 2 > self.queue.len() {
            self.queue.drain(0..self.queue_pos);
            self.queue_pos = 0;
        }
        Ok(())
    }

    fn set_next_step_dir(&mut self, sdir: i32) -> Result<()> {
        if self.sdir == sdir {
            return Ok(());
        }
        self.queue_flush(u64::MAX)?;
        self.sdir = sdir;
        let dir = (sdir ^ self.invert_sdir as i32) != 0;
        self.sink.push(Command::SetNextStepDir(SetNextStepDir {
            oid: self.oid,
            dir,
            req_clock: self.last_step_clock,
        }));
        Ok(())
    }

    fn queue_append_far(&mut self) -> Result<()> {
        let step_clock = self
            .next_step_clock
            .take()
            .expect("pending step clock should exist");
        self.queue_flush(step_clock.saturating_sub(CLOCK_DIFF_MAX).saturating_add(1))?;
        if step_clock >= self.last_step_clock + CLOCK_DIFF_MAX {
            let mv = StepMove {
                interval: (step_clock - self.last_step_clock) as u32,
                count: 1,
                add: 0,
            };
            self.add_move(step_clock, &mv);
            self.calc_last_step_print_time();
            return Ok(());
        }
        self.queue.push(step_clock);
        Ok(())
    }

    fn queue_append_extend(&mut self) -> Result<()> {
        let in_use = self.queue.len() - self.queue_pos;
        if in_use > 65_535 + 2_000 {
            let flush = self.queue[self.queue.len() - 65_535] - self.last_step_clock;
            self.queue_flush(self.last_step_clock + flush)?;
        }

        if self.queue_pos > 0 {
            self.queue.drain(0..self.queue_pos);
            self.queue_pos = 0;
        } else if self.queue.len() == self.queue.capacity() {
            let new_cap = (self.queue.capacity().max(QUEUE_START_SIZE)) * 2;
            self.queue.reserve(new_cap - self.queue.len());
        }
        Ok(())
    }

    fn queue_append(&mut self) -> Result<()> {
        if self.next_step_dir != self.sdir {
            self.set_next_step_dir(self.next_step_dir)?;
        }
        let step_clock = self
            .next_step_clock
            .take()
            .expect("pending step clock should exist");
        if step_clock >= self.last_step_clock + CLOCK_DIFF_MAX {
            self.next_step_clock = Some(step_clock);
            return self.queue_append_far();
        }
        if self.queue.len() == self.queue.capacity() {
            self.queue_append_extend()?;
        }
        self.queue.push(step_clock);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compressor_with_sink() -> StepCompressor<RecordingSink> {
        let sink = RecordingSink::default();
        let mut sc = StepCompressor::new(1, 10, sink);
        sc.set_time(0.0, 1000.0);
        sc
    }

    #[test]
    fn compresses_constant_interval() {
        let mut sc = compressor_with_sink();
        for i in 0..5 {
            sc.append(1, 0.0, i as f64 * 0.001).unwrap();
            sc.commit().unwrap();
        }
        sc.flush(u64::MAX).unwrap();
        let sink = sc.into_sink();
        match &sink.commands[0] {
            Command::SetNextStepDir(_) => {}
            _ => panic!("expected direction setup first"),
        }
        let mut steps = Vec::new();
        for cmd in sink.commands.iter().skip(1) {
            if let Command::QueueStep(step) = cmd {
                steps.push(step);
            }
        }
        let total: u32 = steps.iter().map(|s| s.count as u32).sum();
        assert_eq!(total, 5);
    }

    #[test]
    fn sds_filter_rolls_back_direction_flip() {
        let mut sc = compressor_with_sink();
        // initial step
        sc.append(0, 0.0, 0.0).unwrap();
        // next step very close with opposite dir -> should rollback previous
        sc.append(1, 0.0, 0.0).unwrap();
        sc.commit().unwrap();
        sc.flush(u64::MAX).unwrap();
        let sink = sc.into_sink();
        let total: u32 = sink
            .commands
            .iter()
            .filter_map(|cmd| match cmd {
                Command::QueueStep(step) => Some(step.count as u32),
                _ => None,
            })
            .sum();
        assert_eq!(total, 0);
    }

    #[test]
    fn history_lookup_matches_offset() {
        let mut sc = compressor_with_sink();
        sc.append(1, 0.0, 0.0).unwrap();
        sc.commit().unwrap();
        sc.append(1, 0.0, 0.001).unwrap();
        sc.commit().unwrap();
        sc.flush(u64::MAX).unwrap();
        assert_eq!(sc.last_position(), 2);
        let pos = sc.find_past_position(sc.last_step_clock());
        assert_eq!(pos, 2);
    }
}
