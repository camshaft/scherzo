//! Core motion control utilities shared across Scherzo components.
//!
//! This crate intentionally avoids any transport- or MCU-specific
//! dependencies.

pub mod itersolve;
pub mod kinematics;
pub mod step_compressor;
pub mod trap_queue;
