# zhs

> A self-built Rust framework built around a **thin crates + facade crate** architecture, with zero dependencies in its core.

## Architecture

- **Thin crates** — every capability (JSON, logging, HTTP, scheduling, ...) is a small, independent crate that does one thing.
- **Facade crate** — `zhs-framework` (in `zhs-framework/`) re-exports the thin crates and provides a single entry point.
- **Zero dependencies** — the core pulls in no third-party crates.

## Quick start

```bash
cargo run -p zhs-framework --example hello -- --urls 127.0.0.1:5000
```

```rust
use std::sync::Arc;
use zhs_framework::prelude::*;

struct AppState { greeting: String }

#[injectable]
impl AppState {
    #[inject]
    fn new() -> Self { Self { greeting: "Hello".to_owned() } }
}

struct HelloController { state: Arc<AppState> }

#[controller("/api/[controller]")]
impl HelloController {
    #[inject]
    fn new(state: Arc<AppState>) -> Self { Self { state } }

    #[http_get]
    async fn index(&self) -> ApiResult<String> {
        Ok(format!("{} from zhs", self.state.greeting))
    }
}
```

## Crates

| Crate | Description |
| --- | --- |
| `zhs-framework` | Facade crate (`zhs_framework`); aggregates the sub-crates |
| `zhs-cli` | `zhs new <name>` project generator |
| `zhs-types` | Common value types: `Guid`, `DateTimeOffset`, `Decimal` |
| `zhs-macros` | Internal proc macros (`#[controller]`, `#[injectable]`); re-exported by `zhs-framework` |
| `zhs-json` | JSON parsing and serialization |
| `zhs-log` | Logging |
| `zhs-http` | HTTP/1.1 server (built-in reactor), router, `HttpContext`, responses |
| `zhs-config` | Configuration |
| `zhs-crypto` | Cryptography |
| `zhs-ldap` | LDAP |
| `zhs-mn` | M:N async runtime: `Runtime`, thread pool, timer, watchdog |
| `zhs-scheduler` | Quartz-style job scheduling |
| `zhs-tui` | Terminal UI |
| `zhs-entity` | SQL access: parameterized SQL, `FromRow`, `#[table]`/`#[column]` |
| `zhs-entity-sqlite` | SQLite backend |
| `zhs-entity-mariadb` | MariaDB/MySQL backend |
| `zhs-entity-sqlserver` | SQL Server backend (TDS) |
| `zhs-entity-oracle` | Oracle backend (OCI, runtime-loaded client) |
| `zhs-entity-mongo` | MongoDB backend (BSON/OP_MSG) |
| `zhs-entity-influx` | InfluxDB 2.x backend (line protocol + Flux) |
| `zhs-entity-pi` | PI Web API backend (read-only) |

## Build

Every crate is a member of the Cargo workspace declared in the root `Cargo.toml`, so one command builds all of them:

```bash
cargo build --workspace
```

Build or test a single member with `-p`:

```bash
cargo build -p zhs-json
cargo test -p zhs-json
```

## Documentation

See [doc/readme.md](doc/readme.md) (Chinese).

## Security notes

Read these before deploying:

- **No built-in TLS.** All network clients and servers (HTTP, MariaDB, SQL Server, MongoDB, LDAP, InfluxDB, PI) speak plaintext protocols. SQL Server credentials use the standard TDS obfuscation, which is *not* encryption. Deploy behind a TLS-terminating proxy, or restrict every backend connection to trusted networks.
- **Authorization is opt-in per route.** Controller actions are anonymous unless annotated with `#[authorize]`. There is no global "require authentication by default" switch; audit your routes accordingly.
- **Rate limiting keys on the socket peer address.** `zhs-framework`'s limiter uses the connection's peer IP, not `X-Forwarded-For`. When running behind a proxy, use the trusted-proxy middleware (`zhs-http` `proxy`) so the peer address reflects the real client.
- **CORS:** `allow_any_origin()` never combines with `Access-Control-Allow-Credentials: true` (the combination is ignored by browsers and would reflect arbitrary origins). To send credentials cross-origin, configure an explicit origin allowlist.
- **Crypto** (SHA-1/SHA-256, HMAC, PBKDF2, password hashing, CSPRNG) is implemented from scratch in `zhs-crypto` and validated against RFC/FIPS test vectors, but it has not been independently audited and is not hardened against side-channel attacks beyond constant-time comparisons. Evaluate this against your threat model.

## License

No license has been chosen yet.
