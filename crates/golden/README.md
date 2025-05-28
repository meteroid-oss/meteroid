# Golden

A lightweight Rust library for generating and verifying golden tests for serializable types. This helps ensure backward compatibility as your data structures evolve.

## Overview

Golden tests save serialized versions of your data structures and verify that newer code can still deserialize them. This is particularly useful for:

- Ensuring schema backward compatibility
- Detecting breaking changes in your serialization format
- Documenting how your data structures evolve over time

## Features

- Simple API with minimal boilerplate
- Support for testing multiple variants of each type, and multiple versions
- Works with any type that implements `Serialize` and `Deserialize`

## Usage

### Create tests using the `golden!` macro

Creating a golden test with multiple variants is as simple as calling
`golden!(Type, { variants })`

```rust
use golden::{golden};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
enum Message {
    Text { content: String },
    Image { url: String },
}

// checks against /tests/golden/{type_name}/{ts}-{variant}.json
golden!(Message, {
    "text" => Message::Text { content: "Hello, world!".to_string() }, // "text" variant
    "image" => Message::Image { url: "https://example.com/image.jpg".to_string() }
});

// alternatively, you may provide a custom type name to avoid conflicts
golden!(Message, "my_message", {
    ...
});

```

### Generating Golden Files

The tests will verify against all existing golden files. To generate golden files via CLI, simply run the test with UPDATE_GOLDEN=1

```bash
UPDATE_GOLDEN=1 cargo test

# or for a specific test, ex: if I have golden!(UserToken) in mypackage/src/domain/user_token.rs
UPDATE_GOLDEN=1 cargo test -p mypackage domain::user_token::golden_test_user_token
```

If no golden file content match the one generated from your variant, a new one will be created.

> Note that running with UPDATE_GOLDEN=1 does NOT run the golden tests.

## Directory Structure

Golden files are stored in:
```
your_crate/tests/golden/{type_name}/{timestamp}_{variant}.json
```
