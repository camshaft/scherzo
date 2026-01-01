// Deltesian kinematics

use crate::{
    itersolve::{ActiveFlags, CalcPositionCallback},
    kinematics::move_get_coord,
    trap_queue::Move,
};

/// Deltesian kinematics - hybrid delta/cartesian
pub struct DeltesianKin {
    arm2: f64,
    arm_x: f64,
}

impl DeltesianKin {
    pub fn new(arm2: f64, arm_x: f64) -> Self {
        Self { arm2, arm_x }
    }

    pub fn active_flags(&self) -> ActiveFlags {
        ActiveFlags::new().with_x().with_z()
    }
}

impl CalcPositionCallback for DeltesianKin {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
        let c = move_get_coord(m, move_time);
        let dx = self.arm_x - c.x;
        (self.arm2 - dx * dx).sqrt() + c.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trap_queue::Coord;

    #[test]
    fn deltesian_calculates_position() {
        let mut kin = DeltesianKin::new(100.0, 0.0);
        let m = Move {
            print_time: 0.0,
            move_t: 1.0,
            start_v: 0.0,
            half_accel: 0.0,
            start_pos: Coord {
                x: 0.0,
                y: 0.0,
                z: 5.0,
            },
            axes_r: Coord {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let pos = kin.calc_position(&m, 0.5);
        assert_eq!(pos, 15.0); // sqrt(100) + 5
    }
}
