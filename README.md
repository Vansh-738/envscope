# envscope

Check that environment variables used by Python code are documented in `.env.example`.

A new `os.getenv("PORT")` is easy to add and easy to forget in the example file. envscope catches that mismatch locally or in CI, with a file and line for each lookup.

```text
$ envscope examples/demo
PORT is missing from the example file
  app.py:6:8 (get)
1 Python file(s), 1 undocumented variable(s), 0 dynamic lookup(s), 0 error(s)
```

## Install

Download a binary for Linux (x86-64), macOS (Apple silicon), or Windows (x86-64) from [Releases](https://github.com/Vansh-738/envscope/releases). Each archive includes the executable, README and license. Checksums are included alongside the archives.

To build from source, you need a stable Rust toolchain and a C compiler for the Python grammar.

```sh
cargo install --git https://github.com/Vansh-738/envscope --tag v0.1.0 --locked
```

Or build from a checkout:

```sh
git clone https://github.com/Vansh-738/envscope.git
cd envscope
cargo install --path . --locked
```

## Use

Run in your Python project, or pass its directory:

```sh
envscope
envscope ./backend
```

The example file is relative to the scanned directory. Use another name when needed:

```sh
envscope ./backend --example config/development.env.example
```

Every literal name is checked, including lookups with defaults. A default value does not necessarily mean a setting should be absent from the documentation. Missing names are grouped; each use keeps its own location.

Ignore variables supplied by the operating system, or exclude generated code:

```sh
envscope --ignore-key HOME --ignore-key PATH --exclude 'generated/**'
```

`.gitignore`, `.ignore` and hidden paths are respected, even outside a Git checkout. Symlinks are not followed. Names in the example that have no matching source lookup are left alone.

### Generate a starting example

```sh
envscope --format template > .env.example.new
```

This prints sorted, unique names with empty values. It does not need an existing example file or overwrite one itself. Review the result, particularly lookups reported as dynamic. If a source file cannot be read or parsed, no partial template is printed.

### Machine-readable output

```sh
envscope --format json
```

JSON contains `files_scanned`, `accesses`, `missing`, `unresolved` and `errors`. Each access includes `name`, `file`, `line`, `column` and `kind`. Lines and columns are 1-based; columns count UTF-8 bytes. Paths are relative to the scan directory.

| Exit code | Meaning |
| --- | --- |
| `0` | No undocumented names, or successful template generation |
| `1` | Undocumented names; also dynamic lookups when `--deny-dynamic` is set |
| `2` | Input, parsing, argument or output error |

An input error takes precedence over findings. JSON output still contains findings from other readable files. Dynamic lookups are reported by default but do not fail the check unless `--deny-dynamic` is set.

### GitHub Actions

```yaml
- uses: actions/checkout@v7
- uses: dtolnay/rust-toolchain@stable
- run: cargo install --git https://github.com/Vansh-738/envscope --tag v0.1.0 --locked
- run: envscope ./backend --deny-dynamic
```

## What it recognizes

```python
import os
import os as system
from os import environ as env, getenv as read

os.environ["DATABASE_URL"]
os.environ.get("PORT", "8000")
system.getenv("LOG_LEVEL", "info")
env["API_TOKEN"]
read(key="REGION")
```

Source is parsed with Tree-sitter. Comments, docstrings and ordinary strings do not become lookups. Function imports, import aliases, parameters and ordinary local assignments are accounted for. Simple writes such as `os.environ["TOKEN"] = value` and deletes are excluded; an augmented assignment also reads the old value, so it counts.

Example files support blank lines, comments, `export NAME=value`, a UTF-8 BOM, CRLF and quoted multiline values. Only variable names are retained in the report. Source code is never executed and `.env` is not opened unless explicitly passed as `--example`.

## Limits

This is a static documentation check, not a complete model of Python execution.

- Only `os.getenv`, `os.environ[...]` and `os.environ.get` are recognized. Framework settings classes, wrappers, `getenvb`, wildcard imports and aliases created by assignment are outside the current scope.
- Keys must be literal ASCII environment names: letters, digits and underscores, starting with a letter or underscore. Escaped strings, concatenation, f-strings and computed keys produce an unresolved lookup. Their source expressions are not included in output.
- A name with conflicting imports or assignments in one scope is treated as unknown. Accesses through an unknown binding are skipped; the dynamic count covers recognized environment lookups only.
- Imports are collected per lexical scope. Branch execution, import order, monkey-patching, `global`/`nonlocal` rebinding, pattern-matching captures and every Python scoping edge case are not resolved. In particular, unusual class and comprehension scopes can require review.
- Source files and the example must be UTF-8. Invalid Python files are reported as errors and excluded from analysis.
- The example parser checks names and quote boundaries; it does not expand values or implement every dotenv dialect.

A clean result means no mismatch was found in the supported forms. Keep runtime configuration validation where your application needs it.

## Why another environment tool?

[dotenv-linter](https://github.com/dotenv-linter/dotenv-linter) checks the contents of dotenv files. envscope checks the relationship between Python source and a documented example. It has a narrower job: show which setting was added to code without being added to the example.

## Development

```sh
cargo test --locked
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
```

The test suite covers source recognition, shadowing, dynamic keys, ignored paths, malformed inputs, value-free reports and CLI exit codes. The included demo intentionally has one missing variable so you can try the failure path.

Licensed under the MIT license. See [LICENSE](LICENSE).
