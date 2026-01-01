// Rotary delta kinematics

use crate::{
    itersolve::{ActiveFlags, CalcPositionCallback},
    kinematics::move_get_coord,
    trap_queue::Move,
};

/// Rotary delta kinematics - three rotary arms
pub struct RotaryDeltaKin {
    shoulder_radius: f64,
    shoulder_height: f64,
    angle: f64,
    upper_arm: f64,
    lower_arm: f64,
}

impl RotaryDeltaKin {
    pub fn new(
        shoulder_radius: f64,
        shoulder_height: f64,
        angle: f64,
        upper_arm: f64,
        lower_arm: f64,
    ) -> Self {
        Self {
            shoulder_radius,
            shoulder_height,
            angle,
            upper_arm,
            lower_arm,
        }
    }

    pub fn active_flags(&self) -> ActiveFlags {
        ActiveFlags::new().with_x().with_y().with_z()
    }
}

impl CalcPositionCallback for RotaryDeltaKin {
    fn calc_position(&mut self, m: &Move, move_time: f64) -> f64 {
        let c = move_get_coord(m, move_time);
        // Shoulder coordinates
        let shoulder_x = self.shoulder_radius * self.angle.cos();
        let shoulder_y = self.shoulder_radius * self.angle.sin();

        // Vector from shoulder to effector
        let dx = c.x - shoulder_x;
        let dy = c.y - shoulder_y;
        let dz = c.z - self.shoulder_height;

        // Distance from shoulder to effector
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();

        // Use law of cosines to find shoulder angle
        let cos_angle = (self.upper_arm * self.upper_arm + dist * dist
            - self.lower_arm * self.lower_arm)
            / (2.0 * self.upper_arm * dist);
        let shoulder_angle = cos_angle.acos();

        // Return angle from horizontal
        let horiz_dist = (dx * dx + dy * dy).sqrt();
        let vert_angle = dz.atan2(horiz_dist);
        shoulder_angle + vert_angle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotary_delta_has_all_axes_active() {
        let kin = RotaryDeltaKin::new(50.0, 100.0, 0.0, 100.0, 200.0);
        let flags = kin.active_flags();
        assert!(flags.has_x() && flags.has_y() && flags.has_z());
    }
}
