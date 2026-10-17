//! Pure recipe evaluation and immutable plan data. No filesystem mutation or IO.
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
