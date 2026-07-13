# Contributing to BlackRouter

Thanks for your interest in contributing! This document explains how to get
started.

## Ways to contribute

- Report bugs and request features via [GitHub Issues](https://github.com/AutoWin/blackrouter/issues).
- Improve documentation.
- Submit pull requests for bug fixes, features, or tests.

## Getting started

1. Fork the repository and clone your fork.
2. Install the Rust toolchain (see `rust-toolchain.toml`).
3. Build and run:

   ```bash
   cargo run -p blackrouter-bin
   ```

4. Make your changes on a feature branch.

## Before opening a pull request

Please ensure the following pass locally (they also run in CI):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo build --workspace --release
```

- Keep changes focused and well-described in the PR.
- Add or update tests where it makes sense.
- Update `README.md` / docs when behavior changes.

## Commit messages

Clear, imperative commit messages are appreciated (e.g. `fix(oauth): validate
client redirect_uri`).

## Code of Conduct

By participating, you agree to abide by our
[Code of Conduct](CODE_OF_CONDUCT.md).

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE).
