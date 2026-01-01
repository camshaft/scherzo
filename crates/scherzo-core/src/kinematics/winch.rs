// Winch kinematics

use crate::{
    itersolve::{ActiveFlags, CalcPositionCallback},
    kinematics::move_get_coord,
    trap_queue::Move,
};

/// Winch kinematics - cable-driven system with fixed anchor point
pub struct WinchKin {
    anchor_x: f64,
    anchor_y: f64,
    anchor_z: f64,
}

impl WinchKin {
    pub fn new(anchor_x: f64, anchor_y: f64, anchor_z: f64) -> Self {
        Self {
            anchor_x,
            anchor_y,
            anchor_z,
        }
    }

    pub fn active_flags(&self) -> ActiveFlags {
        ActiveFlags::new().with_x().with_y().with_z()
    }
}

impl CalcPositionCallback for WinchKin {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
        let c = move_get_coord(m, move_time);
        let dx = self.anchor_x - c.x;
        let dy = self.anchor_y - c.y;
        let dz = self.anchor_z - c.z;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trap_queue::Coord;

    #[test]
    fn winch_calculates_cable_length() {
        let mut kin = WinchKin::new(0.0, 0.0, 100.0);
        let m = Move {
            print_time: 0.0,
            move_t: 1.0,
            start_v: 0.0,
            half_accel: 0.0,
            start_pos: Coord {
                x: 3.0,
                y: 4.0,
                z: 0.0,
            },
            axes_r: Coord {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let pos = kin.calc_position(&m, 0.5);
        // sqrt(3^2 + 4^2 + 100^2) = sqrt(10025) ≈ 100.125
        assert!((pos - 100.125).abs() < 0.001);
    }
}
