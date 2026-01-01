// Kinematics systems for various printer types

use crate::trap_queue::{Coord, Move};

// Submodules for each kinematics system
pub mod cartesian;
pub mod corexy;
pub mod corexz;
pub mod delta;
pub mod deltesian;
pub mod extruder;
pub mod generic;
pub mod idex;
pub mod polar;
pub mod rotary_delta;
pub mod shaper;
pub mod winch;

/// Calculate the distance traveled in a move at a given time
pub fn move_get_distance(m: &Move, move_time: f64) -> f64 {
    (m.start_v + m.half_accel * move_time) * move_time
}

/// Calculate the coordinate at a given time in a move
pub fn move_get_coord(m: &Move, move_time: f64) -> Coord {
    let move_dist = move_get_distance(m, move_time);
    Coord {
        x: m.start_pos.x + m.axes_r.x * move_dist,
        y: m.start_pos.y + m.axes_r.y * move_dist,
        z: m.start_pos.z + m.axes_r.z * move_dist,
    }
}
