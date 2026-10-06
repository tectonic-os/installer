# Contributing

The [tectonic-os contributing guide](https://github.com/tectonic-os/.github/blob/main/CONTRIBUTING.md)
sets the rules every repository shares: one feature per pull request, the typed
title, the sign-off, the AI use policy, the code of conduct and where to report
a vulnerability.

## Checks

A pull request merges after these pass, beside GitHub's CodeQL analysis:

- `lint` runs `./lint.sh`: the dependency policy, the boundary greps, rustfmt,
  the tests, and a check that every message names the program.
- `msrv` runs `cargo nextest run --locked --profile ci` on the `rust-version`
  that `Cargo.toml` states.
- `typed-title` checks the pull request title.

Run `./lint.sh --fix` to format, and `./lint.sh` before you push. It is the
check CI runs.
