<p align="center">
  <a href="https://kuroctl.com">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="site/public/assets/dark.png">
      <img src="site/public/assets/light.png" alt="Kuroctl" height="72">
    </picture>
  </a>
</p>

<p align="center">
  <strong>Your game servers, on your machines.</strong><br>
  An open source panel to deploy and run game servers across a fleet of nodes, written in Rust.
</p>

<p align="center">
  <a href="https://www.rust-lang.org"><img alt="Built with Rust" src="https://img.shields.io/badge/BUILT%20WITH-RUST-000000?style=for-the-badge&logo=rust&logoColor=white&labelColor=000000"></a>
  <a href="https://github.com/kuroctl/kuro/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/kuroctl/kuro?style=for-the-badge&label=RELEASE&labelColor=000000&color=f97316"></a>
  <a href="https://github.com/kuroctl/kuro/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/kuroctl/kuro/ci.yml?branch=main&style=for-the-badge&label=CI&labelColor=000000"></a>
  <a href="LICENSE"><img alt="License: AGPL-3.0" src="https://img.shields.io/badge/LICENSE-AGPL--3.0-2563eb?style=for-the-badge&labelColor=000000"></a>
  <a href="https://docs.kuroctl.com"><img alt="Documentation" src="https://img.shields.io/badge/DOCS-DOCS.KUROCTL.COM-4e61ed?style=for-the-badge&labelColor=000000"></a>
</p>

<p align="center">
  <a href="https://docs.kuroctl.com">Documentation</a> ·
  <a href="https://get.kuroctl.com">Install</a> ·
  <a href="https://github.com/kuroctl/kuro/releases">Releases</a>
</p>

---

## Why Kuro

- **One API, many nodes.** The API drives your machines; on each one, a daemon runs Docker. The daemon opens its own encrypted connection to the API (WebSocket over mTLS), so nodes expose no admin port.
- **A prefab for every game.** A server is deployed from a prefab: image, settings, files, install and backup. Write your own in the Studio, try them on a test node, upgrade your servers to the new version.
- **Secure by default.** TLS everywhere, mTLS between the API and nodes, non-root and resource-limited containers, fine-grained permissions and a hash-chained audit log.
- **What you need, nothing more.** Live console, files, backups, schedules, mods, quotas. Optional DNS names per server and an MCP server for AI agents.

## Install

Kuro runs on Linux (x86_64 or arm64) with systemd. One script installs the API and panel, or a node, and checks the minisign signature of every download before installing it.

```bash
curl -fsSLo install.sh https://get.kuroctl.com/api
less install.sh
sudo bash install.sh --public-host panel.example.com --full
```

Then add a node from the panel (**Infrastructure › Nodes › Add a node**) and run the command it shows on the node. The full guide is at **[docs.kuroctl.com](https://docs.kuroctl.com)**.

## How it works

```txt
Panel (browser) ──HTTPS──▶ API ◀──WebSocket mTLS── Daemon (node) ──▶ Docker
Player ───────────────────────────────────────────▶ Node: game port
```

Control and game traffic never cross: players connect straight to the node, so the API is never a bottleneck, and running games carry on if it restarts.

| Folder | Crate | Role |
|---|---|---|
| `api/` | `kuro-api` | Orchestrator: accounts, permissions, nodes, servers, audit (axum, PostgreSQL) |
| `daemon/` | `kuro-daemon` | Agent on each node, drives Docker; `kuro-daemonctl` for maintenance |
| `shared/protocol/` | `kuro-protocol` | Message contract between the API and daemons |
| `shared/prefab/` | `kuro-prefab` | Prefab format and validation |
| `shared/runtime/` | `kuro-runtime` | Common base: logging, graceful shutdown, TLS |
| `panel/` | | Vue 3 admin panel, served by the API |
| `prefabs/` | | Official prefabs |
| `docs/` | | Documentation site (Mintlify), in English and French |
| `site/` | | Install page published at get.kuroctl.com |

## Development

Requirements: Rust stable (1.94 or later), Docker with Compose, and Node.js 24 or later (pnpm through `corepack`).

```bash
cargo xtask dev
```

This starts PostgreSQL, the API, an enrolled daemon and the panel, with output prefixed per process. Ctrl+C stops everything. On first run it creates a dev PKI, an admin account (`admin@example.com`, password in `.kuro-dev/admin.txt`) and a node `dev-1` with ports `25565-25600`. Rust changes rebuild and restart the affected process; the panel reloads through Vite.

Open the panel at `http://localhost:5173`. Use `--no-panel` for the API and daemon only, `--no-reload` to turn off hot reload.

<details>
<summary>Run each process by hand</summary>

```bash
docker compose -f docker-compose.dev.yml up -d
cp .env.example .env

# API PKI, then the API (HTTP on 127.0.0.1:8080, daemon listener on 127.0.0.1:8443)
cargo run -p kuro-api -- --config api/config.example.toml pki init --name localhost --name 127.0.0.1
cargo run -p kuro-api -- --config api/config.example.toml

# Admin account and a node (prints a one-time token and the CA fingerprint)
cargo run -p kuro-api -- --config api/config.example.toml admin create --email admin@example.com --name Admin
cargo run -p kuro-api -- --config api/config.example.toml node add --name dev-1

# Enroll and start the daemon
read -rs KURO_ENROLL_TOKEN && export KURO_ENROLL_TOKEN
cargo run -p kuro-daemon --bin kuro-daemonctl -- --config dev-node/daemon.toml enroll --api https://127.0.0.1:8443 --ca-fingerprint sha256:<fingerprint>
cargo run -p kuro-daemon --bin kuro-daemon -- --config dev-node/daemon.toml

# Panel
corepack pnpm --dir panel install
corepack pnpm --dir panel dev
```

The API's OpenAPI document is served at `/api/openapi.json`; after changing it, regenerate the panel types with `corepack pnpm --dir panel gen:api`. Request collections live in `api/http/`.

</details>

### Checks

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
corepack pnpm --dir panel typecheck
corepack pnpm --dir panel lint
corepack pnpm --dir panel test
```

API tests start PostgreSQL through testcontainers, so Docker is required. SQL queries are checked at compile time against the committed `.sqlx/` cache (`SQLX_OFFLINE=true`). After changing a query or a migration, refresh it:

```bash
sqlx migrate run --source api/migrations
cargo sqlx prepare --workspace -- --all-targets --all-features
```

CI runs all of this on every push, plus `cargo deny` for licenses and security advisories.

## Releases

Pushing a `vX.Y.Z` tag builds static Linux binaries (musl, x86_64 and aarch64), the panel and the prefabs, and publishes them with SHA-256 sums signed by minisign. The install script refuses any archive whose signature or sum does not match.

## License

Kuro is licensed under the [GNU Affero General Public License v3.0](LICENSE).
