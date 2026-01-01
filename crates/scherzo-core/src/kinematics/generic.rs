// Generic cartesian kinematics

use crate::{
    itersolve::{ActiveFlags, CalcPositionCallback},
    kinematics::move_get_coord,
    trap_queue::Move,
};

/// Generic cartesian kinematics with arbitrary coefficients
pub struct GenericCartesianKin {
    a_x: f64,
    a_y: f64,
    a_z: f64,
}

impl GenericCartesianKin {
    pub fn new(a_x: f64, a_y: f64, a_z: f64) -> Self {
        Self { a_x, a_y, a_z }
    }

    pub fn active_flags(&self) -> ActiveFlags {
        let mut flags = ActiveFlags::new();
        if self.a_x != 0.0 {
            flags = flags.with_x();
        }
        if self.a_y != 0.0 {
            flags = flags.with_y();
        }
        if self.a_z != 0.0 {
            flags = flags.with_z();
        }
        flags
    }
}

impl CalcPositionCallback for GenericCartesianKin {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
        let c = move_get_coord(m, move_time);
        self.a_x * c.x + self.a_y * c.y + self.a_z * c.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trap_queue::Coord;

    #[test]
    fn generic_calculates_weighted_sum() {
        let mut kin = GenericCartesianKin::new(1.0, 2.0, 3.0);
        let m = Move {
            print_time: 0.0,
            move_t: 1.0,
            start_v: 0.0,
            half_accel: 0.0,
            start_pos: Coord {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
            axes_r: Coord {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let pos = kin.calc_position(&m, 0.5);
        assert_eq!(pos, 140.0); // 1*10 + 2*20 + 3*30
    }

    #[test]
    fn generic_active_flags_respects_coefficients() {
        let kin = GenericCartesianKin::new(1.0, 0.0, 3.0);
        let flags = kin.active_flags();
        assert!(flags.has_x() && !flags.has_y() && flags.has_z());
    }
}
