# Contributing

Thanks for helping. Setup and the project layout are in the [README](README.md); what's
planned is in [ROADMAP.md](ROADMAP.md).

## Pull requests

`main` is protected: every change goes through a pull request, and the **Rust** and **UI** CI
checks have to pass before it can merge. Before opening one, run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run format:check
npm run check
npm test
```

- If you changed types shared with the UI, run `npm run bindings` and commit the result.
- If you changed Rust dependencies, run `npm run licences` and commit
  `licences/THIRD-PARTY-NOTICES.txt`.
- For UI work, check it in `npm run tauri dev`.
- Move the item to **Done** in `ROADMAP.md` if your change finishes one.

## Bugs and security issues

Open bugs and feature requests through the issue templates. Report security problems
privately, as described in [SECURITY.md](SECURITY.md).
