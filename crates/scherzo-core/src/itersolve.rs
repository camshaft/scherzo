// Iterative solver for kinematic moves

use crate::{
    step_compressor::{CommandSink, StepCompressor},
    trap_queue::{Move, TrapQueue},
};

// Constants
const SEEK_TIME_RESET: f64 = 0.000100;

// Active flags for axis filtering
#[derive(Debug, Clone, Copy, Default)]
pub struct ActiveFlags(u8);

impl ActiveFlags {
    const X: u8 = 1 << 0;
    const Y: u8 = 1 << 1;
    const Z: u8 = 1 << 2;

    pub const fn new() -> Self {
        Self(0)
    }

    pub const fn with_x(mut self) -> Self {
        self.0 |= Self::X;
        self
    }

    pub const fn with_y(mut self) -> Self {
        self.0 |= Self::Y;
        self
    }

    pub const fn with_z(mut self) -> Self {
        self.0 |= Self::Z;
        self
    }

    pub const fn has_x(&self) -> bool {
        self.0 & Self::X != 0
    }

    pub const fn has_y(&self) -> bool {
        self.0 & Self::Y != 0
    }

    pub const fn has_z(&self) -> bool {
        self.0 & Self::Z != 0
    }
}

// Position callback trait - calculates position at a given time in a move
pub trait CalcPositionCallback {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64;
}

// Post-step callback trait - called after steps are generated
pub trait PostCallback {
    fn post_step(&mut self);
}

// Null implementation for when no post-callback is needed
impl PostCallback for () {
    fn post_step(&mut self) {}
}

// Timepos helper struct for secant method
#[derive(Debug, Clone, Copy)]
struct TimePos {
    time: f64,
    position: f64,
}

/// Iterative solver for generating step times from kinematic moves
pub struct IterativeSolver<C, P = ()> {
    step_dist: f64,
    commanded_pos: f64,
    last_flush_time: f64,
    last_move_time: f64,
    active_flags: ActiveFlags,
    gen_steps_pre_active: f64,
    gen_steps_post_active: f64,
    calc_position_cb: C,
    post_cb: P,
}

impl<C: CalcPositionCallback, P: PostCallback> IterativeSolver<C, P> {
    pub fn new(
        step_dist: f64,
        active_flags: ActiveFlags,
        gen_steps_pre_active: f64,
        gen_steps_post_active: f64,
        calc_position_cb: C,
        post_cb: P,
    ) -> Self {
        Self {
            step_dist,
            commanded_pos: 0.0,
            last_flush_time: 0.0,
            last_move_time: 0.0,
            active_flags,
            gen_steps_pre_active,
            gen_steps_post_active,
            calc_position_cb,
            post_cb,
        }
    }

    pub fn commanded_pos(&self) -> f64 {
        self.commanded_pos
    }

    pub fn set_position(&mut self, x: f64, y: f64, z: f64) {
        self.commanded_pos = self.calc_position_from_coord(x, y, z);
    }

    pub fn calc_position_from_coord(&mut self, x: f64, y: f64, z: f64) -> f64 {
        // Create a dummy move at the given position with a long duration
        let m = Move {
            print_time: 0.0,
            move_t: 1000.0,
            start_v: 0.0,
            half_accel: 0.0,
            start_pos: crate::trap_queue::Coord { x, y, z },
            axes_r: crate::trap_queue::Coord {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        self.calc_position_cb.calc_position(&m, 500.0)
    }

    // Check if a move is likely to cause movement on this stepper
    fn check_active(&self, m: &Move) -> bool {
        (self.active_flags.has_x() && m.axes_r.x != 0.0)
            || (self.active_flags.has_y() && m.axes_r.y != 0.0)
            || (self.active_flags.has_z() && m.axes_r.z != 0.0)
    }

    // Generate step times for a portion of a move using secant method
    fn gen_steps_range<S: CommandSink>(
        &mut self,
        sc: &mut StepCompressor<S>,
        m: &Move,
        abs_start: f64,
        abs_end: f64,
    ) -> Result<(), crate::step_compressor::StepCompressError> {
        let half_step = 0.5 * self.step_dist;
        let mut start = abs_start - m.print_time;
        let mut end = abs_end - m.print_time;

        if start < 0.0 {
            start = 0.0;
        }
        if end > m.move_t {
            end = m.move_t;
        }

        let mut old_guess = TimePos {
            time: start,
            position: self.commanded_pos,
        };
        let mut guess = old_guess;
        let mut sdir = sc.get_last_dir();
        let mut is_dir_change = false;
        let mut have_bracket = false;
        let mut check_oscillate = false;
        let mut target = self.commanded_pos + if sdir { half_step } else { -half_step };
        let mut last_time = start;
        let mut low_time = start;
        let mut high_time = start + SEEK_TIME_RESET;
        if high_time > end {
            high_time = end;
        }

        loop {
            // Use the "secant method" to guess a new time from previous guesses
            let guess_dist = guess.position - target;
            let og_dist = old_guess.position - target;
            let mut next_time =
                (old_guess.time * guess_dist - guess.time * og_dist) / (guess_dist - og_dist);

            if !(next_time > low_time && next_time < high_time) {
                // Next guess is outside bounds checks - validate it
                if have_bracket {
                    // A poor guess - fall back to bisection
                    next_time = (low_time + high_time) * 0.5;
                    check_oscillate = false;
                } else if guess.time >= end {
                    // No more steps present in requested time range
                    break;
                } else {
                    // Might be a poor guess - limit to exponential search
                    next_time = high_time;
                    high_time = 2.0 * high_time - last_time;
                    if high_time > end {
                        high_time = end;
                    }
                }
            }

            // Calculate position at next_time guess
            old_guess = guess;
            guess.time = next_time;
            guess.position = self.calc_position_cb.calc_position(m, next_time);
            let guess_dist = guess.position - target;

            if guess_dist.abs() > 0.000000001 {
                // Guess does not look close enough - update bounds
                let rel_dist = if sdir { guess_dist } else { -guess_dist };

                if rel_dist > 0.0 {
                    // Found position past target, so step is definitely present
                    if have_bracket && old_guess.time <= low_time {
                        if check_oscillate {
                            // Force bisect next to avoid persistent oscillations
                            old_guess = guess;
                        }
                        check_oscillate = true;
                    }
                    high_time = guess.time;
                    have_bracket = true;
                } else if rel_dist < -(half_step + half_step + 0.000000010) {
                    // Found direction change
                    sdir = !sdir;
                    target = if sdir {
                        target + half_step + half_step
                    } else {
                        target - half_step - half_step
                    };
                    low_time = last_time;
                    high_time = guess.time;
                    is_dir_change = true;
                    have_bracket = true;
                    check_oscillate = false;
                } else {
                    low_time = guess.time;
                }

                if !have_bracket || high_time - low_time > 0.000000001 {
                    if !is_dir_change && rel_dist >= -half_step {
                        // Avoid rollback if stepper fully reaches step position
                        sc.commit()?;
                    }
                    // Guess is not close enough - guess again with new time
                    continue;
                }
            }

            // Found next step - submit it
            sc.append(sdir as i32, m.print_time, guess.time)?;
            target = if sdir {
                target + half_step + half_step
            } else {
                target - half_step - half_step
            };

            // Reset bounds checking
            let mut seek_time_delta = 1.5 * (guess.time - last_time);
            if seek_time_delta < 0.000000001 {
                seek_time_delta = 0.000000001;
            }
            if is_dir_change && seek_time_delta > SEEK_TIME_RESET {
                seek_time_delta = SEEK_TIME_RESET;
            }
            last_time = guess.time;
            low_time = guess.time;
            high_time = guess.time + seek_time_delta;
            if high_time > end {
                high_time = end;
            }
            is_dir_change = false;
            have_bracket = false;
            check_oscillate = false;
        }

        self.commanded_pos = target - if sdir { half_step } else { -half_step };
        self.post_cb.post_step();
        Ok(())
    }

    // Generate step times for a range of moves on the trapq
    pub fn generate_steps<S: CommandSink>(
        &mut self,
        sc: &mut StepCompressor<S>,
        trapq: &TrapQueue,
        flush_time: f64,
    ) -> Result<(), crate::step_compressor::StepCompressError> {
        let last_flush_time = self.last_flush_time;
        self.last_flush_time = flush_time;

        let moves = trapq.get_active_moves();
        if moves.is_empty() {
            return Ok(());
        }

        // Find first move that hasn't been fully processed
        let mut move_idx = 0;
        while move_idx < moves.len() {
            let m = moves[move_idx];
            if last_flush_time < m.print_time + m.move_t {
                break;
            }
            move_idx += 1;
        }

        if move_idx >= moves.len() {
            return Ok(());
        }

        let mut force_steps_time = self.last_move_time + self.gen_steps_post_active;
        let mut skip_count = 0;

        loop {
            if move_idx >= moves.len() {
                break;
            }

            let m = moves[move_idx];
            let move_start = m.print_time;
            let move_end = move_start + m.move_t;

            if self.check_active(m) {
                if skip_count > 0 && self.gen_steps_pre_active > 0.0 {
                    // Must generate steps leading up to stepper activity
                    let mut abs_start = move_start - self.gen_steps_pre_active;
                    if abs_start < last_flush_time {
                        abs_start = last_flush_time;
                    }
                    if abs_start < force_steps_time {
                        abs_start = force_steps_time;
                    }

                    // Go back and generate steps for skipped moves
                    let mut pm_idx = move_idx;
                    while skip_count > 0 && pm_idx > 0 {
                        pm_idx -= 1;
                        if moves[pm_idx].print_time <= abs_start {
                            pm_idx += 1;
                            break;
                        }
                        skip_count -= 1;
                    }

                    while pm_idx < move_idx {
                        self.gen_steps_range(sc, moves[pm_idx], abs_start, flush_time)?;
                        pm_idx += 1;
                    }
                }

                // Generate steps for this move
                self.gen_steps_range(sc, m, last_flush_time, flush_time)?;

                if move_end >= flush_time {
                    self.last_move_time = flush_time;
                    return Ok(());
                }

                skip_count = 0;
                self.last_move_time = move_end;
                force_steps_time = self.last_move_time + self.gen_steps_post_active;
            } else {
                if move_start < force_steps_time {
                    // Must generate steps just past stepper activity
                    let mut abs_end = force_steps_time;
                    if abs_end > flush_time {
                        abs_end = flush_time;
                    }
                    self.gen_steps_range(sc, m, last_flush_time, abs_end)?;
                    skip_count = 1;
                } else {
                    // This move doesn't impact this stepper - skip it
                    skip_count += 1;
                }
                if flush_time + self.gen_steps_pre_active <= move_end {
                    return Ok(());
                }
            }

            move_idx += 1;
        }

        Ok(())
    }

    // Check if the given stepper is likely to be active in the given time range
    pub fn check_active_time(&self, trapq: &TrapQueue, flush_time: f64) -> Option<f64> {
        let moves = trapq.get_active_moves();
        if moves.is_empty() {
            return None;
        }

        // Find first move past last flush time
        let mut move_idx = 0;
        while move_idx < moves.len() {
            let m = moves[move_idx];
            if self.last_flush_time < m.print_time + m.move_t {
                break;
            }
            move_idx += 1;
        }

        // Check moves for activity
        while move_idx < moves.len() {
            let m = moves[move_idx];
            if self.check_active(m) {
                return Some(m.print_time);
            }
            if flush_time <= m.print_time + m.move_t {
                return None;
            }
            move_idx += 1;
        }

        None
    }

    // Check if this stepper is registered for the given axis
    pub fn is_active_axis(&self, axis: char) -> bool {
        match axis {
            'x' | 'X' => self.active_flags.has_x(),
            'y' | 'Y' => self.active_flags.has_y(),
            'z' | 'Z' => self.active_flags.has_z(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step_compressor::Command;

    // Mock callback that returns a linear position
    struct LinearCallback;

    impl CalcPositionCallback for LinearCallback {
        fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
            // Calculate position along the move using trapezoidal profile
            let move_dist = (m.start_v + m.half_accel * move_time) * move_time;
            m.start_pos.x + m.axes_r.x * move_dist
        }
    }

    // Recording sink for testing
    struct RecordingSink {
        commands: Vec<Command>,
    }

    impl CommandSink for RecordingSink {
        fn push(&mut self, command: Command) {
            self.commands.push(command);
        }
    }

    #[test]
    fn generates_steps_for_linear_motion() {
        let callback = LinearCallback;
        let mut solver = IterativeSolver::new(
            0.1,                         // 0.1mm per step
            ActiveFlags::new().with_x(), // Active on X axis
            0.0,
            0.0,
            callback,
            (),
        );

        let mut trapq = TrapQueue::new();
        trapq.append(
            0.0, // print_time
            0.5, // accel time
            0.5, // cruise time
            0.5, // decel time
            0.0, // start_pos x
            0.0, // start_pos y
            0.0, // start_pos z
            10.0, 10.0, 10.0, // axes_r (x, y, z)
            0.0,  // start_v
            0.0,  // cruise_v
            20.0, // accel
        );

        let sink = RecordingSink {
            commands: Vec::new(),
        };
        let mut sc = StepCompressor::new(0, 1000, sink);
        sc.set_time(0.0, 1_000_000.0); // 1 MHz MCU clock

        solver
            .generate_steps(&mut sc, &trapq, 1.5)
            .expect("generate_steps failed");

        let commands = sc.into_sink().commands;

        // With 10 mm/s velocity and 0.1mm steps, we expect steps
        // The exact count depends on the move profile, but there should be some
        assert!(!commands.is_empty(), "Expected some step commands");
    }

    #[test]
    fn detects_direction_changes() {
        struct OscillatingCallback;

        impl CalcPositionCallback for OscillatingCallback {
            fn calc_position(&mut self, _m: &Move, move_time: f64) -> f64 {
                // Oscillate back and forth
                (move_time * 10.0).sin() * 2.0
            }
        }

        let callback = OscillatingCallback;
        let mut solver =
            IterativeSolver::new(0.1, ActiveFlags::new().with_x(), 0.0, 0.0, callback, ());

        let mut trapq = TrapQueue::new();
        trapq.append(
            0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        );

        let sink = RecordingSink {
            commands: Vec::new(),
        };
        let mut sc = StepCompressor::new(0, 1000, sink);
        sc.set_time(0.0, 1_000_000.0); // 1 MHz MCU clock

        solver
            .generate_steps(&mut sc, &trapq, 1.0)
            .expect("generate_steps failed");

        let commands = sc.into_sink().commands;

        // Count direction changes
        let mut dir_changes = 0;
        for command in &commands {
            if matches!(command, Command::SetNextStepDir(_)) {
                dir_changes += 1;
            }
        }

        // Should have multiple direction changes due to oscillation
        assert!(
            dir_changes > 1,
            "Expected multiple direction changes, got {}",
            dir_changes
        );
    }

    #[test]
    fn respects_axis_filtering() {
        let callback = LinearCallback;
        let mut solver = IterativeSolver::new(
            0.1,
            ActiveFlags::new().with_y(), // Only active on Y
            0.0,
            0.0,
            callback,
            (),
        );

        let mut trapq = TrapQueue::new();
        // Add move with only X motion
        trapq.append(
            0.0, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0, 10.0, 0.0, 0.0, 0.0, 0.0, 20.0,
        );

        let sink = RecordingSink {
            commands: Vec::new(),
        };
        let mut sc = StepCompressor::new(0, 1000, sink);
        sc.set_time(0.0, 1_000_000.0); // 1 MHz MCU clock

        solver
            .generate_steps(&mut sc, &trapq, 1.5)
            .expect("generate_steps failed");

        let commands = sc.into_sink().commands;

        // Should have no steps since motion is on X but stepper is only active on Y
        assert_eq!(commands.len(), 0, "Expected no commands for filtered axis");
    }

    #[test]
    fn calculates_position_from_coordinates() {
        struct CoordCallback;

        impl CalcPositionCallback for CoordCallback {
            fn calc_position(&mut self, m: &Move, _move_time: f64) -> f64 {
                // Return X + 2*Y + 3*Z as the position
                m.start_pos.x + 2.0 * m.start_pos.y + 3.0 * m.start_pos.z
            }
        }

        let mut solver = IterativeSolver::new(
            0.1,
            ActiveFlags::new().with_x(),
            0.0,
            0.0,
            CoordCallback,
            (),
        );

        let pos = solver.calc_position_from_coord(1.0, 2.0, 3.0);
        assert_eq!(pos, 1.0 + 2.0 * 2.0 + 3.0 * 3.0); // 1 + 4 + 9 = 14
    }
}
