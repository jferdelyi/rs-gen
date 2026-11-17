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
#![doc(
	html_logo_url = "https://raw.githubusercontent.com/jferdelyi/rs-gen/main/assets/rs-gen-icon.svg",
	html_favicon_url = "https://raw.githubusercontent.com/jferdelyi/rs-gen/main/assets/rs-gen-icon.svg"
)]

/// Core n-gram models and generation logic.
///
/// This module exposes the high-level generator interface while keeping
/// internal model representations private.
pub mod model;

/// I/O utilities (file loading, path helpers).
///
/// Not exposed
pub(crate) mod io;

/// Fast hasher for the internal tables.
///
/// Not exposed
pub(crate) mod hash;
