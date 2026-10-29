# Contributing

Bug reports are most useful with a small Python snippet, an example file with empty values, the command you ran, and the output you expected. Leave credentials and private source code out of reports.

For a parser change, add a regression test in `tests/audit.rs`. Run the checks listed in the README before opening a pull request.

The current scope is Python's standard `os` environment APIs. Please open an issue before adding a framework integration or another language; those need a separate design decision.
