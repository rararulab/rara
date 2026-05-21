# Non-Rust Toolchain — mise

The non-Rust dev toolchain (Node, Bun, Go, `just`, `gh`) is pinned via
[mise](https://mise.jdx.dev) in `mise.toml` at the repo root. The same
`mise.toml` is consumed by `jdx/mise-action@v2` in CI, so local and CI
versions stay in lockstep.

```bash
brew install mise              # one-time
mise install                   # provisions everything in mise.toml
echo 'eval "$(mise activate zsh)"' >> ~/.zshrc   # one-time shell hook
```

After activation, `cd`-ing into the repo selects the pinned versions
automatically; outside the repo your system tools are untouched. `mise`
is **warn-only** in `./init.sh` — rustup-only contributors working on
pure Rust changes can skip it.

Rust stays on rustup: `rust-toolchain.toml` pins stable and `cargo +nightly`
comes from rustup. `prek` (pre-commit) stays on brew and `diesel_cli`
stays on `cargo install` — mise's cargo backend would recompile both
from source on every `mise install` for no upside.

See `mise.toml` for the version pins and the rationale comment.
