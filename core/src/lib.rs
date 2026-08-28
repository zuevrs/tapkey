//! The tapkey engine.
//!
//! Deliberately empty. This crate exists from the first commit so that CI compiles it —
//! including for Linux — rather than asserting portability it never checks. The engine
//! itself is built one decision at a time; nothing is added here ahead of the decision
//! that shapes it.
//!
//! The shape it grows into is fixed: one function, a JSON request in and a JSON response
//! out, behind a versioned schema, with three consumers sharing it.

pub mod atomic;
pub mod json;

/// The schema version carried by every request and response.
///
/// Zero until the schema exists. It is here so that the first request cannot be written
/// without deciding what version it declares.
pub const SCHEMA_VERSION: u32 = 0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crate_compiles_and_declares_an_unstable_schema() {
        assert_eq!(SCHEMA_VERSION, 0, "bump this deliberately, with the schema");
    }
}
