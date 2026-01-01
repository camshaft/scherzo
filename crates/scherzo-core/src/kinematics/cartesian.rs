// Cartesian kinematics - standard X/Y/Z motion

use crate::{
    itersolve::{ActiveFlags, CalcPositionCallback},
    kinematics::move_get_coord,
    trap_queue::Move,
};

/// Which axis this stepper controls
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    /// Parse axis from string (case-insensitive)
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "x" => Some(Axis::X),
            "y" => Some(Axis::Y),
            "z" => Some(Axis::Z),
            _ => None,
        }
    }
}

/// Cartesian kinematics - each stepper directly controls one axis
pub struct CartesianKin {
    axis: Axis,
}

impl CartesianKin {
    pub fn new(axis: Axis) -> Self {
        Self { axis }
    }

    pub fn active_flags(&self) -> ActiveFlags {
        match self.axis {
            Axis::X => ActiveFlags::new().with_x(),
            Axis::Y => ActiveFlags::new().with_y(),
            Axis::Z => ActiveFlags::new().with_z(),
        }
    }
}

impl CalcPositionCallback for CartesianKin {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
        let c = move_get_coord(m, move_time);
        match self.axis {
            Axis::X => c.x,
            Axis::Y => c.y,
            Axis::Z => c.z,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trap_queue::Coord;

    #[test]
    fn axis_parse() {
        assert_eq!(Axis::parse("x"), Some(Axis::X));
        assert_eq!(Axis::parse("X"), Some(Axis::X));
        assert_eq!(Axis::parse("y"), Some(Axis::Y));
        assert_eq!(Axis::parse("z"), Some(Axis::Z));
        assert_eq!(Axis::parse("w"), None);
    }

    #[test]
    fn cartesian_x_calculates_x_position() {
        let mut kin = CartesianKin::new(Axis::X);
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
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let pos = kin.calc_position(&m, 0.5);
        assert_eq!(pos, 10.0);
    }

    #[test]
    fn cartesian_active_flags() {
        assert!(CartesianKin::new(Axis::X).active_flags().has_x());
        assert!(!CartesianKin::new(Axis::X).active_flags().has_y());
        assert!(CartesianKin::new(Axis::Y).active_flags().has_y());
        assert!(CartesianKin::new(Axis::Z).active_flags().has_z());
    }
}
