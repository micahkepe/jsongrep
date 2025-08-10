/*!
This module provides tokenization of JSON strings. It is currently unused by the rest of the
codebase, but may be useful in the future in constructing the document tree without external
dependencies. Currently, the JSON is deserialized into a `serde_json::Value` using
`serde_json::from_str`, which is not ideal for performance reasons as it requires the entire JSON
string to be deserialized. Instead, in the future, we may use a streaming deserializer to
deserialize the JSON string into a stream of tokens.

The tokenization is done by the [`lexer`] module, which is responsible for
lexing the JSON string into a stream of [`JToken`]s.

[`lexer`]: lexer
[`JToken`]: token::JToken

# Example

```rust
use jsongrep::tokenizer::{lexer::tokenize, token::JToken};

let json = br#"
{
    "name": "John Doe",
    "age": 30,
    "address": {
        "street": "123 Main St",
        "city": "Anytown",
        "state": "CA",
        "zip": 12345
    }
}
"#;

let (tokens, bytes_read) = tokenize(json);
```
*/
pub mod lexer;
pub mod token;

// Re-exports
pub use lexer::tokenize;
pub use token::JToken;
