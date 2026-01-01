// Polar kinematics

use crate::{
    itersolve::{ActiveFlags, CalcPositionCallback, PostCallback},
    kinematics::move_get_coord,
    trap_queue::Move,
};

/// Polar axis type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolarAxis {
    /// Radius axis
    Radius,
    /// Angle axis
    Angle,
}

impl PolarAxis {
    /// Parse polar axis from string
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "r" | "radius" => Some(PolarAxis::Radius),
            "a" | "angle" => Some(PolarAxis::Angle),
            _ => None,
        }
    }
}

/// Polar kinematics - bed rotates and arm moves radially
pub struct PolarKin {
    axis: PolarAxis,
    #[allow(dead_code)] // TODO: Used for angle unwrapping in post_step
    last_angle: f64,
}

impl PolarKin {
    pub fn new(axis: PolarAxis) -> Self {
        Self {
            axis,
            last_angle: 0.0,
        }
    }

    pub fn active_flags(&self) -> ActiveFlags {
        ActiveFlags::new().with_x().with_y()
    }
}

impl CalcPositionCallback for PolarKin {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
        let c = move_get_coord(m, move_time);
        match self.axis {
            PolarAxis::Radius => (c.x * c.x + c.y * c.y).sqrt(),
            PolarAxis::Angle => c.y.atan2(c.x),
        }
    }
}

impl PostCallback for PolarKin {
    fn post_step(&mut self) {
        // Track angle for unwrapping after steps are generated
        // Note: In the original C code, this tracks the last angle seen
        // For now, we just update the tracking variable
        // This would be called after itersolve processes each move
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trap_queue::Coord;

    #[test]
    fn polar_axis_parse() {
        assert_eq!(PolarAxis::parse("r"), Some(PolarAxis::Radius));
        assert_eq!(PolarAxis::parse("radius"), Some(PolarAxis::Radius));
        assert_eq!(PolarAxis::parse("a"), Some(PolarAxis::Angle));
        assert_eq!(PolarAxis::parse("angle"), Some(PolarAxis::Angle));
        assert_eq!(PolarAxis::parse("x"), None);
    }

    #[test]
    fn polar_radius_calculates_distance() {
        let mut kin = PolarKin::new(PolarAxis::Radius);
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
        assert_eq!(pos, 5.0); // sqrt(3^2 + 4^2)
    }

    #[test]
    fn polar_angle_calculates_atan2() {
        let mut kin = PolarKin::new(PolarAxis::Angle);
        let m = Move {
            print_time: 0.0,
            move_t: 1.0,
            start_v: 0.0,
            half_accel: 0.0,
            start_pos: Coord {
                x: 1.0,
                y: 0.0,
                z: 0.0,
            },
            axes_r: Coord {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        };
        let pos = kin.calc_position(&m, 0.5);
        assert_eq!(pos, 0.0); // atan2(0, 1) = 0
    }
}
