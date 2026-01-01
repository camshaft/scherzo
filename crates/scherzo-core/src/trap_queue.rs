//! Trapezoidal velocity movement queue.
//!
//! A faithful, safe port of Klipper's `trapq` helper. It tracks active
//! trapezoid segments (accel/cruise/decel), fills gaps with optional
//! null moves for numerical stability, maintains history, and can
//! expose both in-flight and historical moves for diagnostics.

use std::collections::VecDeque;

const NEVER_TIME: f64 = 9_999_999_999_999_999.9;
const MAX_NULL_MOVE: f64 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Coord {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Move {
    pub print_time: f64,
    pub move_t: f64,
    pub start_v: f64,
    pub half_accel: f64,
    pub start_pos: Coord,
    pub axes_r: Coord,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PullMove {
    pub print_time: f64,
    pub move_t: f64,
    pub start_v: f64,
    pub accel: f64,
    pub start_x: f64,
    pub start_y: f64,
    pub start_z: f64,
    pub x_r: f64,
    pub y_r: f64,
    pub z_r: f64,
}

fn move_get_distance(m: &Move, move_time: f64) -> f64 {
    (m.start_v + m.half_accel * move_time) * move_time
}

fn move_get_coord(m: &Move, move_time: f64) -> Coord {
    let move_dist = move_get_distance(m, move_time);
    Coord {
        x: m.start_pos.x + m.axes_r.x * move_dist,
        y: m.start_pos.y + m.axes_r.y * move_dist,
        z: m.start_pos.z + m.axes_r.z * move_dist,
    }
}

#[allow(dead_code)]
fn copy_pull_move(p: &mut PullMove, m: &Move) {
    p.print_time = m.print_time;
    p.move_t = m.move_t;
    p.start_v = m.start_v;
    p.accel = 2.0 * m.half_accel;
    p.start_x = m.start_pos.x;
    p.start_y = m.start_pos.y;
    p.start_z = m.start_pos.z;
    p.x_r = m.axes_r.x;
    p.y_r = m.axes_r.y;
    p.z_r = m.axes_r.z;
}

pub struct TrapQueue {
    moves: VecDeque<Move>, // includes head and tail sentinels
    history: VecDeque<Move>,
}

impl Default for TrapQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl TrapQueue {
    pub fn new() -> Self {
        let mut moves = VecDeque::new();
        // Head sentinel
        moves.push_back(Move {
            print_time: -1.0,
            ..Move::default()
        });
        // Tail sentinel
        moves.push_back(Move {
            print_time: NEVER_TIME,
            move_t: NEVER_TIME,
            ..Move::default()
        });
        Self {
            moves,
            history: VecDeque::new(),
        }
    }

    fn tail_index(&self) -> usize {
        self.moves.len() - 1
    }

    fn head_index(&self) -> usize {
        0
    }

    fn tail_mut(&mut self) -> &mut Move {
        let idx = self.tail_index();
        self.moves.get_mut(idx).expect("tail sentinel")
    }

    /// Update the tail sentinel's print_time and start_pos if it's marked stale.
    pub fn check_sentinels(&mut self) {
        let tail_idx = self.tail_index();
        if self.moves[tail_idx].print_time != 0.0 {
            return;
        }
        let prev_idx = tail_idx - 1;
        if prev_idx == self.head_index() {
            self.moves[tail_idx].print_time = NEVER_TIME;
            self.moves[tail_idx].move_t = NEVER_TIME;
            return;
        }
        let prev = self.moves[prev_idx];
        let tail = self.tail_mut();
        tail.print_time = prev.print_time + prev.move_t;
        tail.move_t = 0.0;
        tail.start_pos = move_get_coord(&prev, prev.move_t);
    }

    /// Add a fully-prepared move, filling gaps with a null move when necessary.
    pub fn add_move(&mut self, m: Move) {
        let tail_idx = self.tail_index();
        let prev = self.moves[tail_idx - 1];
        if prev.print_time + prev.move_t < m.print_time {
            let mut null_move = Move {
                start_pos: m.start_pos,
                ..Move::default()
            };
            if prev.print_time <= 0.0 && m.print_time > MAX_NULL_MOVE {
                null_move.print_time = m.print_time - MAX_NULL_MOVE;
            } else {
                null_move.print_time = prev.print_time + prev.move_t;
            }
            null_move.move_t = m.print_time - null_move.print_time;
            let insert_at = self.tail_index();
            self.moves.insert(insert_at, null_move);
        }
        let insert_at = self.tail_index();
        self.moves.insert(insert_at, m);
        // mark tail stale so check_sentinels recomputes
        let tail = self.tail_mut();
        tail.print_time = 0.0;
        tail.move_t = 0.0;
    }

    /// Convenience builder mirroring the C `trapq_append` helper.
    #[allow(clippy::too_many_arguments)]
    pub fn append(
        &mut self,
        print_time: f64,
        accel_t: f64,
        cruise_t: f64,
        decel_t: f64,
        start_pos_x: f64,
        start_pos_y: f64,
        start_pos_z: f64,
        axes_r_x: f64,
        axes_r_y: f64,
        axes_r_z: f64,
        start_v: f64,
        cruise_v: f64,
        accel: f64,
    ) {
        let mut cur_time = print_time;
        let mut cur_pos = Coord {
            x: start_pos_x,
            y: start_pos_y,
            z: start_pos_z,
        };
        let axes_r = Coord {
            x: axes_r_x,
            y: axes_r_y,
            z: axes_r_z,
        };

        if accel_t > 0.0 {
            let m = Move {
                print_time: cur_time,
                move_t: accel_t,
                start_v,
                half_accel: 0.5 * accel,
                start_pos: cur_pos,
                axes_r,
            };
            self.add_move(m);
            cur_time += accel_t;
            cur_pos = move_get_coord(&m, accel_t);
        }

        if cruise_t > 0.0 {
            let m = Move {
                print_time: cur_time,
                move_t: cruise_t,
                start_v: cruise_v,
                half_accel: 0.0,
                start_pos: cur_pos,
                axes_r,
            };
            self.add_move(m);
            cur_time += cruise_t;
            cur_pos = move_get_coord(&m, cruise_t);
        }

        if decel_t > 0.0 {
            let m = Move {
                print_time: cur_time,
                move_t: decel_t,
                start_v: cruise_v,
                half_accel: -0.5 * accel,
                start_pos: cur_pos,
                axes_r,
            };
            self.add_move(m);
        }
    }

    /// Expire any moves older than `print_time`, moving them into history.
    pub fn finalize_moves(&mut self, print_time: f64, clear_history_time: f64) {
        while self.moves.len() > 2 {
            let m = self.moves[1];
            if m.print_time + m.move_t > print_time {
                break;
            }
            let moved = self.moves.remove(1).unwrap();
            if moved.start_v != 0.0 || moved.half_accel != 0.0 {
                self.history.push_front(moved);
            }
        }

        if self.moves.len() == 2 {
            let tail = self.tail_mut();
            tail.print_time = NEVER_TIME;
            tail.move_t = NEVER_TIME;
        }

        if let Some(latest) = self.history.front().cloned() {
            while self.history.len() > 1 {
                let last = *self.history.back().unwrap();
                if last.print_time + last.move_t > clear_history_time {
                    break;
                }
                if last == latest {
                    break;
                }
                self.history.pop_back();
            }
        }
    }

    /// Note a position change; flush pending moves and mark a history entry.
    pub fn set_position(&mut self, print_time: f64, pos_x: f64, pos_y: f64, pos_z: f64) {
        self.finalize_moves(NEVER_TIME, 0.0);

        while let Some(first) = self.history.front_mut() {
            if first.print_time < print_time {
                if first.print_time + first.move_t > print_time {
                    first.move_t = print_time - first.print_time;
                }
                break;
            }
            self.history.pop_front();
        }

        self.history.push_front(Move {
            print_time,
            start_pos: Coord {
                x: pos_x,
                y: pos_y,
                z: pos_z,
            },
            ..Move::default()
        });
    }

    /// Return in-flight and historical moves that overlap the given time window.
    pub fn extract_old(&self, max: usize, start_time: f64, end_time: f64) -> Vec<PullMove> {
        let mut result = Vec::new();

        // Iterate active moves (skip head sentinel at index 0, tail sentinel at len-1)
        for i in (1..self.moves.len() - 1).rev() {
            let m = &self.moves[i];
            if m.print_time > end_time {
                continue;
            }
            if m.print_time + m.move_t < start_time {
                break;
            }
            result.push(PullMove {
                print_time: m.print_time,
                move_t: m.move_t,
                start_v: m.start_v,
                accel: 2.0 * m.half_accel,
                start_x: m.start_pos.x,
                start_y: m.start_pos.y,
                start_z: m.start_pos.z,
                x_r: m.axes_r.x,
                y_r: m.axes_r.y,
                z_r: m.axes_r.z,
            });
            if result.len() >= max {
                break;
            }
        }

        // Iterate history moves
        for m in self.history.iter().rev() {
            if m.print_time > end_time {
                continue;
            }
            if m.print_time + m.move_t < start_time {
                break;
            }
            result.push(PullMove {
                print_time: m.print_time,
                move_t: m.move_t,
                start_v: m.start_v,
                accel: 2.0 * m.half_accel,
                start_x: m.start_pos.x,
                start_y: m.start_pos.y,
                start_z: m.start_pos.z,
                x_r: m.axes_r.x,
                y_r: m.axes_r.y,
                z_r: m.axes_r.z,
            });
            if result.len() >= max {
                break;
            }
        }

        result
    }

    /// Get active moves as references (for itersolve)
    /// Returns moves between start and end sentinels
    pub fn get_active_moves(&self) -> Vec<&Move> {
        if self.moves.len() <= 2 {
            Vec::new()
        } else {
            // Skip head sentinel at 0 and tail sentinel at len-1
            self.moves.range(1..self.moves.len() - 1).collect()
        }
    }

    /// Get history moves as references
    pub fn get_history_moves(&self) -> Vec<&Move> {
        self.history.iter().collect()
    }

    /// Current active moves (excluding sentinels). Useful for tests/inspection.
    pub fn active_len(&self) -> usize {
        self.moves.len().saturating_sub(2)
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn tail_sentinel(&self) -> Move {
        *self.moves.back().expect("tail sentinel")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_segments_and_updates_sentinel() {
        let mut tq = TrapQueue::new();
        tq.append(
            0.0, 1.0, 2.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0,
        );
        assert_eq!(tq.active_len(), 4); // initial null move + 3 segments
        tq.check_sentinels();
        let tail = tq.tail_sentinel();
        assert!(tail.print_time > 0.0);
    }

    #[test]
    fn inserts_null_move_for_gap() {
        let mut tq = TrapQueue::new();
        let m1 = Move {
            print_time: 0.0,
            move_t: 0.5,
            ..Move::default()
        };
        tq.add_move(m1);
        let m2 = Move {
            print_time: 2.0,
            move_t: 0.5,
            ..Move::default()
        };
        tq.add_move(m2);
        assert_eq!(tq.active_len(), 4); // initial null + m1 + gap null + m2
    }

    #[test]
    fn finalizes_into_history() {
        let mut tq = TrapQueue::new();
        tq.append(
            0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 0.0, 1.0,
        );
        tq.finalize_moves(2.0, 0.0);
        assert_eq!(tq.active_len(), 0);
        assert!(tq.history_len() >= 1);
        tq.check_sentinels();
        assert_eq!(tq.tail_sentinel().print_time, NEVER_TIME);
    }

    #[test]
    fn extract_includes_active_and_history() {
        let mut tq = TrapQueue::new();
        tq.append(
            0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
        );

        // Before finalize, should have null move + actual move
        let pulled = tq.extract_old(4, 0.0, 2.0);
        assert_eq!(pulled.len(), 2, "Should have null move + actual move");

        tq.finalize_moves(2.0, 0.0);

        // After finalize, null moves are filtered (they have start_v=0, half_accel=0)
        let pulled2 = tq.extract_old(4, 0.0, 2.0);
        assert_eq!(pulled2.len(), 1, "Null moves filtered from history");
    }

    #[test]
    fn set_position_truncates_history() {
        let mut tq = TrapQueue::new();
        tq.append(
            0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0,
        );
        tq.finalize_moves(2.0, 0.0);
        tq.set_position(0.25, 1.0, 2.0, 3.0);
        assert!(tq.history_len() >= 1);
        let marker = tq.history.front().unwrap();
        assert_eq!(marker.print_time, 0.25);
        assert_eq!(marker.start_pos.x, 1.0);
    }
}
