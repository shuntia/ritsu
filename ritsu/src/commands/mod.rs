//! Command modules

#![allow(clippy::unnecessary_wraps)]

pub mod daemon;
pub mod send;
pub mod trigger;
pub mod task;
pub mod memory;

#[cfg(feature = "gui")]
pub mod chat;
