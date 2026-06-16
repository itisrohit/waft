#![allow(
    dead_code,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::unnecessary_debug_formatting
)]

pub mod cli;
pub mod clipboard;
pub mod daemon;
pub mod discovery;
pub mod error;
pub mod identity;

pub mod send;
pub mod transfer;
pub mod trust;
