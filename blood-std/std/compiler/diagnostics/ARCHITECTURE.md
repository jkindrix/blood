# Blood Diagnostics Module Architecture

## Overview

This document outlines the architecture for Blood's diagnostic reporting system,
informed by best practices from Swift, Clang, rustc, ariadne, and miette.

## Design Principles

### 1. Message Format (from Swift)

- Single phrase or sentence, **no terminal period**
- Use semicolons to separate distinct ideas
- Omit filler words that don't add information
- Phrase as **rules, not failures**: prefer `'super.init' cannot be called outside`
  over `cannot call 'super.init' outside`

### 2. Spans (from rustc)

- Use the **smallest span** that still signifies the issue
- Primary spans should be **self-explanatory** for IDE display
- Support multi-line spans for complex errors

### 3. Suggestions (from Clang/rustc)

- Fix-its must be **obvious, singular, and highly likely correct**
- **Must not change code meaning** for warnings
- Include **applicability levels** for tool integration

### 4. Terminology (from rustc)

- Use **backticks** for code identifiers: `` `foo.bar` ``
- Use "invalid" not "illegal"
- Avoid unnecessarily alarming language

---

## Module Structure

```
std/compiler/diagnostics/
├── mod.blood              # Module entry point and re-exports
├── severity.blood         # Severity levels (Error, Warning, Note, Help, Remark)
├── span.blood             # Source span and location tracking
├── code.blood             # Error codes registry and metadata
├── diagnostic.blood       # Core Diagnostic type with builder pattern
├── label.blood            # Labels for annotating source spans
├── suggestion.blood       # Suggestions and fix-its
├── related.blood          # Related diagnostics (multiple errors)
├── group.blood            # Diagnostic groups for categorization
├── context.blood          # DiagnosticContext for emission management
├── emitter/
│   ├── mod.blood          # Emitter trait and common types
│   ├── terminal.blood     # Terminal/ANSI emitter with colors
│   ├── json.blood         # JSON output for IDE/tool integration
│   └── plain.blood        # Plain text (no colors, no Unicode)
├── renderer/
│   ├── mod.blood          # Renderer trait
│   ├── source.blood       # Source file management
│   ├── snippet.blood      # Source code snippet extraction
│   └── annotate.blood     # Source annotation with underlines/labels
└── ice.blood              # Internal Compiler Error infrastructure
```

---

## Core Types

### Severity Levels

```blood
pub enum Severity {
    /// Compilation-blocking error
    Error,

    /// Potentially problematic but compiles
    Warning,

    /// Contextual information attached to error/warning
    Note,

    /// Actionable suggestion for fixing
    Help,

    /// Informational message (compilation progress, etc.)
    Remark,
}
```

### Error Codes

Error codes follow a structured organization:

| Range | Category | Examples |
|-------|----------|----------|
| E0001-E0099 | Lexer errors | Unexpected character, unclosed string |
| E0100-E0199 | Parser errors | Unexpected token, missing delimiter |
| E0200-E0299 | Name resolution | Undefined identifier, ambiguous name |
| E0300-E0399 | Type errors | Type mismatch, invalid conversion |
| E0400-E0499 | Effect errors | Unhandled effect, invalid handler |
| E0500-E0599 | Pattern errors | Non-exhaustive match, unreachable pattern |
| E0600-E0699 | Codegen errors | Invalid FFI, unsupported target |
| E9000-E9999 | ICE | Internal compiler errors |
| W0001-W0099 | Memory warnings | Deep nesting, excessive indirection |
| W0100-W0199 | Style warnings | Unused variable, dead code |

Each error code has:
- Unique identifier (e.g., `E0100`)
- Short description
- Long explanation (for `--explain E0100`)
- Optional help text
- Diagnostic group membership

### Applicability Levels

```blood
pub enum Applicability {
    /// Safe for automated tools to apply without human review
    MachineApplicable,

    /// Contains placeholders requiring user input (e.g., `<type>`)
    HasPlaceholders,

    /// Might not be correct; use with caution
    MaybeIncorrect,

    /// Applicability unknown; conservative default
    Unspecified,
}
```

### Labels

```blood
pub struct Label {
    /// Source span this label annotates
    span: Span,

    /// Message to display at this location
    message: Option[String],

    /// Whether this is the primary label
    primary: Bool,

    /// Priority for ordering when labels overlap (lower = higher priority)
    priority: U32,
}
```

### Suggestions

```blood
pub struct Suggestion {
    /// Human-readable description of what this suggestion does
    message: String,

    /// The code change to apply
    substitution: Substitution,

    /// How confident we are in this suggestion
    applicability: Applicability,
}

pub enum Substitution {
    /// Insert text at a position
    Insert { position: usize, text: String },

    /// Remove a span of text
    Remove { span: Span },

    /// Replace a span with new text
    Replace { span: Span, replacement: String },
}
```

### Core Diagnostic

```blood
pub struct Diagnostic {
    /// Severity level
    severity: Severity,

    /// Error code (optional, e.g., "E0100")
    code: Option[ErrorCode],

    /// Primary error message
    message: String,

    /// Primary span where the error occurred
    span: Span,

    /// Additional labels pointing to relevant code
    labels: Vec[Label],

    /// Suggestions for fixing the error
    suggestions: Vec[Suggestion],

    /// Related diagnostics (sub-errors, notes)
    related: Vec[Diagnostic],

    /// Diagnostic group for categorization
    group: Option[DiagnosticGroup],
}
```

---

## Diagnostic Context

The `DiagnosticContext` manages diagnostic emission and state:

```blood
pub struct DiagnosticContext {
    /// Accumulated diagnostics
    diagnostics: Vec[Diagnostic],

    /// Error count (for early termination)
    error_count: USize,

    /// Warning count
    warning_count: USize,

    /// Maximum errors before stopping (0 = unlimited)
    max_errors: USize,

    /// Emitter for output
    emitter: Box[dyn Emitter],

    /// Source files for snippet rendering
    sources: SourceMap,

    /// Whether to treat warnings as errors
    warnings_as_errors: Bool,

    /// Suppressed warning groups
    suppressed_groups: HashSet[DiagnosticGroup],
}
```

Key methods:
- `emit(diagnostic)` - Emit and record a diagnostic
- `struct_err(message, span)` - Create an error builder
- `struct_warn(message, span)` - Create a warning builder
- `has_errors()` - Check if any errors occurred
- `abort_if_errors()` - Abort compilation if errors present

---

## Emitter Trait

```blood
pub trait Emitter {
    /// Emit a diagnostic to the output
    fn emit(self, diagnostic: Diagnostic, sources: SourceMap) -> Unit

    /// Flush any buffered output
    fn flush(mut self) -> Unit

    /// Check if this emitter supports colors
    fn supports_color(self) -> Bool
}
```

### Terminal Emitter

Features:
- ANSI color codes (red for errors, yellow for warnings, etc.)
- Unicode box-drawing for source snippets
- Smart label positioning to avoid overlaps
- Configurable: color on/off, Unicode on/off, tab width

### JSON Emitter

For IDE integration (LSP, etc.):
```json
{
  "severity": "error",
  "code": "E0100",
  "message": "unexpected token",
  "span": {
    "file": "src/main.blood",
    "start": {"line": 10, "column": 5},
    "end": {"line": 10, "column": 8}
  },
  "labels": [...],
  "suggestions": [...]
}
```

### Plain Emitter

For CI/logging where colors aren't supported:
```
error[E0100]: unexpected token
 --> src/main.blood:10:5
   |
10 |     let = 42;
   |         ^ expected identifier
   |
```

---

## Renderer Architecture

### Source Management

```blood
pub struct SourceMap {
    /// Map of file ID to source content
    files: HashMap[FileId, SourceFile],
}

pub struct SourceFile {
    /// File name/path
    name: String,

    /// Full source content
    content: String,

    /// Line start indices for O(log n) lookup
    line_starts: Vec[usize],
}
```

### Snippet Rendering

The renderer extracts relevant source lines and annotates them:

```
error[E0300]: type mismatch
  --> src/main.blood:15:12
   |
15 |     let x: Int = "hello"
   |            ---   ^^^^^^^ expected `Int`, found `String`
   |            |
   |            expected due to this
   |
help: consider using `parse` to convert the string
   |
15 |     let x: Int = "hello".parse()
   |                         ++++++++
```

Features:
- Line numbers with proper padding
- Multi-line span support
- Overlapping label handling
- Suggestion diff display
- Context lines (configurable)

---

## Internal Compiler Errors (ICE)

ICEs indicate bugs in the compiler, not user code:

```blood
pub struct Ice {
    /// What went wrong
    message: String,

    /// Compiler source file where ICE occurred
    file: String,

    /// Line number in compiler source
    line: U32,

    /// Additional context
    context: Vec[String],
}
```

ICE output:
```
internal compiler error: type variable should have been resolved

  --> bloodc/src/typeck/unify.blood:234

This is a bug in the Blood compiler. Please report it at:
https://github.com/blood-lang/blood/issues

Context:
  - type_var: T42
  - during: unification
```

---

## Diagnostic Groups

Groups allow categorizing related diagnostics:

```blood
pub enum DiagnosticGroup {
    /// Lexical analysis issues
    Lexer,

    /// Syntax/parsing issues
    Syntax,

    /// Name resolution issues
    Resolution,

    /// Type checking issues
    Types,

    /// Effect system issues
    Effects,

    /// Pattern matching issues
    Patterns,

    /// Memory/pointer warnings
    Memory,

    /// Performance warnings
    Performance,

    /// Style/convention warnings
    Style,

    /// Unused code warnings
    Unused,
}
```

Usage:
- `--warn-error Types` - Treat type warnings as errors
- `--allow Unused` - Suppress unused code warnings
- `--explain Types` - Show documentation for the group

---

## Usage Examples

### Creating a Simple Error

```blood
let diagnostic = Diagnostic::error("unexpected token", span)
    .with_code(ErrorCode::UnexpectedToken)
    .with_label(Label::primary(span, "expected identifier"))
    .with_suggestion(Suggestion::replace(
        span,
        "variable_name",
        Applicability::HasPlaceholders,
    ))

ctx.emit(diagnostic)
```

### Creating an Error with Related Diagnostics

```blood
let main_error = Diagnostic::error("type mismatch", expr_span)
    .with_code(ErrorCode::TypeMismatch)
    .with_label(Label::primary(expr_span, "expected `Int`, found `String`"))
    .with_label(Label::secondary(type_span, "expected due to this"))
    .with_related(
        Diagnostic::note("the type `String` cannot be converted to `Int`", expr_span)
    )
    .with_related(
        Diagnostic::help("use `.parse()` to convert strings to integers", expr_span)
            .with_suggestion(Suggestion::insert(
                expr_span.end,
                ".parse()",
                Applicability::MaybeIncorrect,
            ))
    )

ctx.emit(main_error)
```

### ICE Macro

```blood
// Simple ICE
ice!("type variable should have been resolved before codegen")

// ICE with context
ice!("unexpected type kind"; "type" => ty, "expected" => "function")

// ICE that returns a Result
ice_err!(span, "mismatched field count"; "expected" => n, "found" => m)
```

---

## Implementation Plan

### Phase 1: Core Types (~800 LOC)
1. `severity.blood` - Severity enum
2. `span.blood` - Span type (port from Rust)
3. `code.blood` - ErrorCode enum with metadata
4. `label.blood` - Label type
5. `suggestion.blood` - Suggestion and Substitution types
6. `diagnostic.blood` - Core Diagnostic with builder

### Phase 2: Rendering (~600 LOC)
1. `renderer/source.blood` - SourceMap and SourceFile
2. `renderer/snippet.blood` - Snippet extraction
3. `renderer/annotate.blood` - Annotation rendering

### Phase 3: Emitters (~500 LOC)
1. `emitter/mod.blood` - Emitter trait
2. `emitter/terminal.blood` - ANSI terminal output
3. `emitter/plain.blood` - Plain text output
4. `emitter/json.blood` - JSON output

### Phase 4: Infrastructure (~400 LOC)
1. `context.blood` - DiagnosticContext
2. `group.blood` - DiagnosticGroup
3. `related.blood` - Related diagnostics
4. `ice.blood` - ICE infrastructure

### Phase 5: Integration (~300 LOC)
1. `mod.blood` - Module entry point with re-exports
2. Integration with lexer, parser, typeck
3. Comprehensive tests

---

## References

- [Swift Diagnostics](https://github.com/apple/swift/blob/main/docs/Diagnostics.md)
- [Clang Internals - Diagnostics](https://clang.llvm.org/docs/InternalsManual.html)
- [rustc Diagnostics Guide](https://rustc-dev-guide.rust-lang.org/diagnostics.html)
- [ariadne](https://docs.rs/ariadne/latest/ariadne/)
- [miette](https://docs.rs/miette/latest/miette/)
