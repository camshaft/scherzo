// Delta kinematics

use crate::{
    itersolve::{ActiveFlags, CalcPositionCallback},
    kinematics::move_get_coord,
    trap_queue::Move,
};

/// Delta kinematics - three vertical towers with arms to effector
pub struct DeltaKin {
    arm2: f64,
    tower_x: f64,
    tower_y: f64,
}

impl DeltaKin {
    pub fn new(arm2: f64, tower_x: f64, tower_y: f64) -> Self {
        Self {
            arm2,
            tower_x,
            tower_y,
        }
    }

    pub fn active_flags(&self) -> ActiveFlags {
        ActiveFlags::new().with_x().with_y().with_z()
    }
}

impl CalcPositionCallback for DeltaKin {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
        let c = move_get_coord(m, move_time);
        let dx = self.tower_x - c.x;
        let dy = self.tower_y - c.y;
        (self.arm2 - dx * dx - dy * dy).sqrt() + c.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trap_queue::Coord;

    #[test]
    fn delta_calculates_tower_height() {
        let mut kin = DeltaKin::new(100.0, 0.0, 0.0);
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
