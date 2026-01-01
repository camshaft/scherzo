// CoreXY kinematics

use crate::{
    itersolve::{ActiveFlags, CalcPositionCallback},
    kinematics::move_get_coord,
    trap_queue::Move,
};

/// CoreXY stepper type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepperType {
    /// Plus stepper: position = X + Y
    Plus,
    /// Minus stepper: position = X - Y
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

/// CoreXY kinematics - two motors control X and Y with belt arrangement
pub struct CoreXYKin {
    stepper_type: StepperType,
}

impl CoreXYKin {
    pub fn new(stepper_type: StepperType) -> Self {
        Self { stepper_type }
    }

    pub fn active_flags(&self) -> ActiveFlags {
        ActiveFlags::new().with_x().with_y()
    }
}

impl CalcPositionCallback for CoreXYKin {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
        let c = move_get_coord(m, move_time);
        match self.stepper_type {
            StepperType::Plus => c.x + c.y,
            StepperType::Minus => c.x - c.y,
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
    fn corexy_plus_sums_x_and_y() {
        let mut kin = CoreXYKin::new(StepperType::Plus);
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
        assert_eq!(pos, 30.0); // 10 + 20
    }

    #[test]
    fn corexy_minus_diffs_x_and_y() {
        let mut kin = CoreXYKin::new(StepperType::Minus);
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
        assert_eq!(pos, -10.0); // 10 - 20
    }
}
