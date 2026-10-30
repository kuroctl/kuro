# Kuro

Panel de gestion de serveurs de jeu multi-nœuds (inspiré de Pterodactyl), backend entièrement en Rust.

## Composants

| Dossier | Crate | Binaires |
|---|---|---|
| `api/` | `kuro-api` | `kuro-api` : orchestrateur (axum, PostgreSQL) |
| `daemon/` | `kuro-daemon` | `kuro-daemon` : agent sur chaque hôte ; `kuro-daemonctl` : CLI de maintenance |
| `shared/protocol/` | `kuro-protocol` | contrat de messages API ↔ daemon |
| `shared/runtime/` | `kuro-runtime` | socle commun : logs, arrêt propre, TLS |
| `panel/` | — | SPA Vue 3 du panel admin |

Organisation du code : [ADR-0004](docs/decisions/ADR-0004-organisation-du-code.md). Lien API ↔ daemon : [ADR-0005](docs/decisions/ADR-0005-lien-daemon-implementation.md). Enrôlement : [ADR-0006](docs/decisions/ADR-0006-enrolement-csr.md). Auth admin et RBAC : [ADR-0007](docs/decisions/ADR-0007-auth-admin-rbac.md). Audit : [ADR-0008](docs/decisions/ADR-0008-audit.md). Panel : [ADR-0009](docs/decisions/ADR-0009-panel-front.md). Régions : [ADR-0010](docs/decisions/ADR-0010-regions-placement.md). Prefabs : [ADR-0011](docs/decisions/ADR-0011-prefabs.md). Erreurs : [ADR-0012](docs/decisions/ADR-0012-erreurs-problem-json.md). Serveurs : [ADR-0013](docs/decisions/ADR-0013-serveurs.md).

## Démarrage (dev)

Prérequis : Rust stable (≥ 1.94), Docker, Docker Compose, et `sqlx-cli` pour toucher aux requêtes SQL :

```bash
cargo install sqlx-cli --no-default-features --features postgres,rustls
```

### En une commande

Prérequis en plus : Node.js ≥ 24 (pnpm via `corepack`).

```bash
cargo xtask dev
```

Lance Postgres (docker compose), l'API, un daemon enrôlé et le panel, avec des sorties préfixées par process. Ctrl+C arrête tout. `--no-panel` pour l'API et le daemon seuls.

Rechargement à chaud : une modification des sources Rust (`api/`, `daemon/`, `shared/`, `.sqlx/`, migrations) recompile et relance le process concerné, l'API appliquant ses nouvelles migrations à son démarrage ; le panel a celui de Vite. Une compilation ratée laisse tourner la version en cours. `--no-reload` pour s'en passer.

Au premier lancement, l'initialisation est automatique et idempotente :
- PKI de dev (`dev-pki/`) ;
- compte super-admin `admin@example.com`, mot de passe généré dans `.kuro-dev/admin.txt` (ignoré par git, jamais affiché ; un compte existant n'est pas modifié) ;
- région `eu-west` et nœud `dev-1` enrôlé dans `dev-node/`, avec les ports `25565-25600` pour ses serveurs (un `dev-node/` dont le nœud a été révoqué ou supprimé est déplacé dans `.kuro-dev/stale-dev-node/` puis réenrôlé) ;
- dépendances du panel.

Les prefabs de `prefabs/` sont importés au démarrage de l'API, puis dès que le dossier change (un prefab déjà importé sous le même id et la même version n'est jamais remplacé : changer son contenu demande une nouvelle version). Les données des serveurs du daemon de dev vont dans `.kuro-dev/servers/`.

Panel : `http://localhost:5173`. Les étapes suivantes détaillent le même démarrage, process par process.

### 1. Base de données

```bash
docker compose -f docker-compose.dev.yml up -d
cp .env.example .env    # DATABASE_URL pour sqlx-cli (migrate, prepare)
```

Les migrations (`api/migrations/`) sont appliquées au démarrage de l'API.

### 2. PKI de l'API

Crée la CA et le certificat serveur du listener daemon dans `dev-pki/` (ne régénère jamais l'existant) :

```bash
cargo run -p kuro-api -- --config api/config.example.toml pki init --name localhost --name 127.0.0.1
```

### 3. API

```bash
cargo run -p kuro-api -- --config api/config.example.toml
```

L'API écoute en HTTP sur `127.0.0.1:8080`. Le listener daemon (`127.0.0.1:8443`, TLS de la CA privée) sert le lien WebSocket mTLS et l'enrôlement.

### 4. Ajouter et enrôler un nœud

Sur l'hôte de l'API : crée le nœud `pending` et affiche son token à usage unique (15 min) et l'empreinte de la CA. La région est facultative.

```bash
cargo run -p kuro-api -- --config api/config.example.toml region add --slug eu-west --name "Europe Ouest" --latitude 48.85 --longitude 2.35
```

```bash
cargo run -p kuro-api -- --config api/config.example.toml node add --name dev-1 --region eu-west --display-name "Dev 1"
```

Token perdu ou expiré : `node token --name dev-1`. Révocation : `node revoke --name dev-1` (réenrôlement par un nouveau token) ; suppression : `node delete --name dev-1 --yes`. Même chose par le REST : `api/http/admin-nodes.http`.

Sur l'hôte du nœud : coller le token quand il est demandé (il n'apparaît ni à l'écran ni dans l'historique), puis enrôler.

```bash
read -rs KURO_ENROLL_TOKEN && export KURO_ENROLL_TOKEN
```

```bash
cargo run -p kuro-daemon --bin kuro-daemonctl -- --config dev-node/daemon.toml enroll --api https://127.0.0.1:8443 --ca-fingerprint sha256:<empreinte>
```

La clé est générée localement ; `dev-node/` reçoit `node.key`, `node.pem`, `ca.pem` et `daemon.toml`.

### 5. Compte admin

Sur l'hôte de l'API (mot de passe demandé sans écho, ou `--password-stdin` pour un script) :

```bash
cargo run -p kuro-api -- --config api/config.example.toml admin create --email admin@example.com --name Admin
```

La config de dev désactive le TOTP (`auth.require_totp = false`) : la connexion se fait par mot de passe seul. En production, le TOTP est configuré à la première connexion. Récupération : `admin reset-password` et `admin reset-totp`.

Collections de requêtes : `api/http/admin-auth.http`, `api/http/admin-iam.http`, `api/http/admin-nodes.http`.

### 6. Daemon

```bash
cargo run -p kuro-daemon --bin kuro-daemon -- --config dev-node/daemon.toml
```

Le nœud passe `online` dans la table `nodes`.

Configuration : `api/config.example.toml`, surchargeable par variables `KURO_API_<SECTION>__<CLÉ>` (ex. `KURO_API_HTTP__BIND`).

### 7. Panel

Prérequis : Node.js ≥ 24 et pnpm (via `corepack`). L'API doit tourner : le serveur de dev de Vite relaie `/api` vers `127.0.0.1:8080`, sur la même origine que le panel (cookie de rafraîchissement `SameSite=Strict`).

```bash
corepack pnpm --dir panel install
```

```bash
corepack pnpm --dir panel dev
```

Panel sur `http://localhost:5173`, connexion avec le compte admin créé à l'étape 5. Après toute modification de l'OpenAPI de l'API, régénérer les types du panel :

```bash
corepack pnpm --dir panel gen:api
```

### Documentation de l'API

- OpenAPI (JSON) : `GET /api/openapi.json`, ou `cargo run -p kuro-api -- openapi > openapi.json`.
- Collection de requêtes : `api/http/*.http` (client HTTP JetBrains, httpyac, Kulala…), environnement `dev` dans `api/http/http-client.env.json`.

### CLI de maintenance du daemon

```bash
cargo run -p kuro-daemon --bin kuro-daemonctl -- --config dev-node/daemon.toml config check
```

### Journal d'audit

```bash
cargo run -p kuro-api -- --config api/config.example.toml audit verify
```

```bash
cargo run -p kuro-api -- --config api/config.example.toml audit purge
```

`verify` recalcule la chaîne de hash (code de sortie non nul si le journal a été altéré). `purge` applique la durée de conservation des paramètres du projet ; l'API le fait déjà elle-même chaque jour. Le panel (Paramètres → Journal d'audit) filtre, vérifie et exporte le journal en CSV.

## Qualité

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Les tests sont des tests de fonctionnalités, dans `<crate>/tests/features/`. Ceux de l'API démarrent un Postgres via testcontainers (Docker requis), conteneur `kuro-test-postgres` réutilisé d'un lancement à l'autre.

Les requêtes `sqlx::query!` sont vérifiées à la compilation contre le cache commité `.sqlx/` : `SQLX_OFFLINE=true` est posé par `.cargo/config.toml`, donc la compilation ne dépend pas de l'état de la base de dev. Après toute modification de requête ou de migration, appliquer les migrations à la base de dev puis régénérer le cache (`cargo sqlx prepare` interroge la base de `DATABASE_URL`, fichier `.env`) :

```bash
sqlx migrate run --source api/migrations
cargo sqlx prepare --workspace -- --all-targets --all-features
```

Détails et erreurs typiques : [docs/developpement.md](docs/developpement.md). Pour vérifier une requête en cours d'écriture contre la base sans régénérer le cache, `SQLX_OFFLINE=false cargo check`. Un cache `.sqlx/` oublié fait échouer la compilation (« no cached data for this query ») : relancer `cargo sqlx prepare`.

Panel : typage, lint (dont aucun texte en dur dans les templates) et tests de features (Vitest, Testing Library, API simulée par MSW, dans `panel/tests/features/`) :

```bash
corepack pnpm --dir panel typecheck
corepack pnpm --dir panel lint
corepack pnpm --dir panel test
```

Dépendances (`deny.toml` : licences, sources, avis de sécurité) et scripts d'installation : voir [docs/developpement.md](docs/developpement.md). La CI (`.github/workflows/ci.yml`) lance tout cela à chaque push.

## Installation (production)

Sur tag `vX.Y.Z`, `.github/workflows/release.yml` publie les binaires Linux (musl, x86_64 et aarch64), le panel et les prefabs, avec des sommes SHA-256 signées par minisign (ADR-0016). Sur les hôtes, `scripts/install.sh` (ADR-0028) vérifie la signature et les sommes avant d'installer un rôle, `api` ou `node`, et prépare un serveur vierge avec `--full` :

```bash
sudo bash install.sh api --public-host panel.example.com --full
```

```bash
KURO_ENROLL_TOKEN=… sudo -E bash install.sh node --api https://panel.example.com:8443 --ca-fingerprint sha256:… --ports 25565-25600 --full
```

Guide complet : [docs.kuroctl.com](https://docs.kuroctl.com) (Mintlify, sources dans `docs/`). La page d'installation get.kuroctl.com vient de `site/`.

L'API sert le panel (`http.panel_dir`) et écoute en local : un reverse proxy TLS (Caddy) la publie. Avant la première release : créer la paire de clés (`minisign -G`), mettre la clé privée dans les secrets GitHub (`MINISIGN_SECRET_KEY`, `MINISIGN_PASSWORD`), la clé publique dans `scripts/install.sh`, et fixer `KURO_REPO`.

## Documentation

- `CLAUDE.md` : conventions et périmètre
- `docs/architecture.md` : vue d'ensemble
- `docs/decisions/` : ADR
- `docs/developpement.md` : pièges connus (sqlx hors ligne, migrations, prefabs, panel)
- `docs/roadmap.md` : fonctionnalités prévues

## Statut

Premier lot : workspace ✅, protocole `hello`/`heartbeat` ✅, connexion d'un nœud en mTLS ✅, enrôlement par CSR ✅. Auth admin (TOTP, RBAC) et audit ✅. Nœuds et régions en REST, révocation ✅. Erreurs RFC 9457 traduisibles ✅. Panel : socle, connexion et compte ✅, comptes admin et rôles ✅. Serveurs : prefabs (ADR-0011), allocations de ports, déploiement durci par le daemon, power actions, réglages générés dans le panel ✅ (ADR-0013). Console et stats en temps réel, commandes auditées ✅ (ADR-0014). Fichiers des volumes (liste, éditeur, conflits, `openat2` sous Linux) ✅ (ADR-0015). CI, publication signée, scripts d'installation, panel servi par l'API ✅ (ADR-0016). Installation, réinstallation et réinitialisation des serveurs (téléchargements vérifiés, extraction confinée, SteamCMD et scripts isolés) ✅ (ADR-0017). Sauvegardes à chaud, rotation, restauration et téléchargement ✅ (ADR-0018). Paramètres du projet, journal d'audit dans le panel, ressources à chaud, quota disque et suspension ✅ (ADR-0019). Mods, plugins et modpacks Modrinth avec leurs dépendances, envoi de fichiers ✅ (ADR-0020).
