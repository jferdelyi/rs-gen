//! N-gram-based text generation library.
//!
//! This crate provides a modular n-gram generation system including:
//! - Character-level n-gram models
//! - Multi-size (multi-order) n-gram aggregation
//! - Probabilistic generation with controllable randomness
//! - Internal utilities for I/O and path handling
//!
//! Only the high-level API is exposed publicly. Low-level components
//! are kept internal to ensure consistency and prevent misuse.

/// Core n-gram models and generation logic.
///
/// This module exposes the high-level generator interface while keeping
/// internal model representations private.
pub mod model;

/// I/O utilities (file loading, path helpers).
///
/// Not exposed
pub(crate) mod io;
