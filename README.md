# P4 Language Server

A Language Server Protocol (LSP) implementation for the P4 programming language, built on top of the [Oxide Computer P4 compiler](https://github.com/oxidecomputer/p4).

## Features

- **Diagnostics**: Real-time error and warning reporting from the P4 compiler
- **Hover**: Type information and documentation for headers, structs, controls, parsers, actions, and externs
- **Completion**: Auto-completion for P4 keywords, types, and user-defined symbols

## Installation

### From source

```bash
cd p4-lsp
cargo build --release
cp target/release/p4-lsp ~/.local/bin/  # or anywhere in your PATH
```

## Usage

The language server communicates over stdio using the Language Server Protocol.

### With Zed

Install the `zed-p4-language` extension and ensure `p4-lsp` is in your PATH.

### Configuration (Zed)

In your Zed settings, you can configure the LSP binary path:

```json
{
  "lsp": {
    "p4-lsp": {
      "binary": {
        "path": "/path/to/p4-lsp"
      }
    }
  }
}
```

## Architecture

The LSP uses the P4 compiler's:
- **Lexer/Parser**: For parsing P4 source with full source location tracking
- **Type Checker**: For semantic analysis and diagnostics
- **AST**: For symbol information (hover, completion)

## License

MPL-2.0
