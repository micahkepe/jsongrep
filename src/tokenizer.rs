//! # Tokenizer/ Lexer
//!
//! Parses an input sequence of bytes from a JSON document into a token stream.
pub mod lexer;
pub mod token;

// Re-exports
pub use lexer::tokenize;
pub use token::JToken;
