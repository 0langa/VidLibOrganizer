//! Pure recipe evaluation and immutable plans, with a separate read-only input preparation stage.
pub mod evaluate;
pub mod model;
pub mod recipe;
pub use evaluate::evaluate;
pub use model::*;
pub use recipe::*;

pub mod graph;
pub use graph::plan;

pub mod validate;

pub mod text;
pub use text::*;

pub mod regex;
pub use regex::*;

pub mod numbers;
pub use numbers::*;
pub mod dates;
pub use dates::*;

pub mod template;
pub use template::*;

pub mod hashes;
pub use hashes::*;
