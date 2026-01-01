// CoreXZ kinematics

use crate::{
    itersolve::{ActiveFlags, CalcPositionCallback},
    kinematics::move_get_coord,
    trap_queue::Move,
};

/// CoreXZ stepper type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepperType {
    /// Plus stepper: position = X + Z
    Plus,
    /// Minus stepper: position = X - Z
    Minus,
}

impl StepperType {
    /// Parse stepper type from string
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "+" | "plus" => Some(StepperType::Plus),
            "-" | "minus" => Some(StepperType::Minus),
            _ => None,
        }
    }
}

/// CoreXZ kinematics - two motors control X and Z with belt arrangement
pub struct CoreXZKin {
    stepper_type: StepperType,
}

impl CoreXZKin {
    pub fn new(stepper_type: StepperType) -> Self {
        Self { stepper_type }
    }

    pub fn active_flags(&self) -> ActiveFlags {
        ActiveFlags::new().with_x().with_z()
    }
}

impl CalcPositionCallback for CoreXZKin {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
        let c = move_get_coord(m, move_time);
        match self.stepper_type {
            StepperType::Plus => c.x + c.z,
            StepperType::Minus => c.x - c.z,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trap_queue::Coord;

    #[test]
    fn stepper_type_parse() {
        assert_eq!(StepperType::parse("+"), Some(StepperType::Plus));
        assert_eq!(StepperType::parse("plus"), Some(StepperType::Plus));
        assert_eq!(StepperType::parse("-"), Some(StepperType::Minus));
        assert_eq!(StepperType::parse("minus"), Some(StepperType::Minus));
        assert_eq!(StepperType::parse("x"), None);
    }

    #[test]
    fn corexz_plus_sums_x_and_z() {
        let mut kin = CoreXZKin::new(StepperType::Plus);
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
        assert_eq!(pos, 40.0); // 10 + 30
    }

    #[test]
    fn corexz_minus_diffs_x_and_z() {
        let mut kin = CoreXZKin::new(StepperType::Minus);
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
        assert_eq!(pos, -20.0); // 10 - 30
    }
}
