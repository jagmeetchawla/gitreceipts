# Third-party notices

gitreceipts is MIT-licensed (see `LICENSE`). It builds on the work of others,
acknowledged here.

## Secret & PII detection (`src/scan.rs`)

The built-in scanner's detection patterns and false-positive-rejection logic
were informed by five open-source secret scanners. Provider token patterns are
published credential *grammars* (facts — e.g. a GitHub token is `ghp_` + 36
chars, dictated by GitHub); validators (Luhn, IBAN mod-97, the distinct-character
placeholder filter) were reimplemented from their described algorithms, not
copied. These projects are credited with gratitude:

- **gitleaks** — MIT License, Copyright (c) 2019 Zachary Rice.
  https://github.com/gitleaks/gitleaks
- **Kingfisher** — Apache License 2.0, Copyright (c) MongoDB, Inc.
  https://github.com/mongodb/kingfisher
- **Nosey Parker** — Apache License 2.0, Copyright (c) Praetorian Security, Inc.
  https://github.com/praetorian-inc/noseyparker
- **ripsecrets** — MIT License, Copyright (c) 2022 Brian Smith.
  https://github.com/sirwart/ripsecrets
- **Velka** — MIT OR Apache-2.0, Copyright (c) Wesllen Lima.
  https://github.com/wesllen-lima/velka

## Runtime dependencies

gitreceipts links the following crates, all under permissive licenses
(MIT and/or Apache-2.0). The authoritative list, with versions, is in
`Cargo.toml` / `Cargo.lock`; each dependency ships its own license text via
crates.io.

- `anyhow`, `chrono`, `clap`, `regex`, `serde`, `serde_json` (and their
  transitive dependencies) — MIT OR Apache-2.0.
- `libc` (Unix SIGPIPE handling) — MIT OR Apache-2.0.

A complete, generated notice for a binary distribution (e.g. via `cargo about`)
will accompany released binaries.
