//! Top-level module for the n-gram generation system.
//!
//! This crate provides a multi-level n-gram text generator, including:
//! - Fixed-order n-gram models (`NGramModel`)
//! - Multi-size combined models (`MultiGramModel`)
//! - Internal state management (`State`)
//! - Prediction configuration (`PredictionInput`)
//! - A high-level generation interface (`Generator`)

/// High-level interface for generating sequences using one or more n-gram models.
///
/// Exposes model loading, weighted model selection, and sequence generation
/// with configurable randomness and seeding.
pub mod generator;

/// Multi-size n-gram model composed of multiple `NGramModel`s.
///
/// Supports loading from disk, parallel construction, merging,
/// and probabilistic character prediction.
pub mod multigram_model;

/// Fixed-order n-gram model (`n >= 2`).
///
/// Handles sentence ingestion, transition counting,
/// probabilistic next-character prediction, and model merging.
pub mod ngram_model;

/// Internal representation of a single n-gram state (prefix).
///
/// Tracks outgoing transitions and supports weighted random sampling.
/// This module is not exposed publicly.
mod state;

/// Internal prediction configuration structure.
///
/// Stores generation parameters such as randomness, retry limits,
/// model intensities, and normalized model selection probabilities.
/// Used internally by `Generator`.
pub mod prediction_input;
