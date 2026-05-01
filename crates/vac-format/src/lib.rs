//! VAC source format: AST, text writer, and parser.
//!
//! The AST in [`ast`] is the single shared contract between
//! `vac-compiler` (AST → video) and `vac-decompiler` (video → AST).
//!
//! - [`writer`] turns an AST into well-formed `.vac` text.
//! - [`parser`] turns `.vac` text into an AST.
//!
//! Round-trip invariant:  `parse(write(ast)) == ast` for any AST that
//! the writer can emit.

pub mod ast;
pub mod parser;
pub mod writer;

pub use ast::*;
pub use parser::{parse, ParseError};
pub use writer::write;
