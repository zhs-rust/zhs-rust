# zhs

> A self-built Rust framework built around a **thin crates + facade crate** architecture, with zero dependencies in its core.

## Architecture

- **Thin crates** — every capability (JSON, logging, HTTP, scheduling, ...) is a small, independent crate that does one thing.
- **Facade crate** — `zhs-framework` re-exports the thin crates and provides a single entry point.
- **Zero dependencies** — the core pulls in no third-party crates.

## Crates

| Crate | Description |
| --- | --- |
| `zhs-framework` | Facade crate; aggregates the sub-crates |
| `zhs-json` | JSON parsing and serialization |
| `zhs-log` | Logging |
| `zhs-http` | HTTP |
| `zhs-config` | Configuration |
| `zhs-crypto` | Cryptography |
| `zhs-ldap` | LDAP |
| `zhs-mn` | Model / microservice building blocks |
| `zhs-scheduler` | Scheduling |
| `zhs-tui` | Terminal UI |
| `zhs-entity` | Entity abstraction |
| `zhs-entity-sqlite` / `-mariadb` / `-mongo` / `-oracle` / `-sqlserver` / `-influx` / `-pi` | Entity backends for each database |

## Build

Each crate is currently standalone, so build it from its own directory:

```bash
cd zhs-json
cargo build
```

There is no workspace manifest at the repository root yet; add one if you want a single `cargo build` for every crate.

## Documentation

See [doc/readme.md](doc/readme.md) (Chinese).

## License

No license has been chosen yet.
