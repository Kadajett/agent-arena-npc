//! Temporary compatibility belongs here and must be feature-gated.
//!
//! Backend-reported combat actions remain authoritative. The
//! `compat-hardcoded-skills` feature is reserved for a narrowly scoped adapter
//! if a live gateway payload still lacks that data during migration.

pub const HARDCODED_SKILLS_ENABLED: bool = cfg!(feature = "compat-hardcoded-skills");
