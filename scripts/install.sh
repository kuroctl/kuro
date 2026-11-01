#!/usr/bin/env bash
# Installe ou met à jour Kuro sur un hôte Linux (ADR-0016, ADR-0028).
#
#   Rôles : api (API, panel, prefabs), node (daemon). Un ou les deux.
#   --full : prépare d'abord un serveur vierge (mises à jour, pare-feu, SSH, fail2ban, Docker,
#            Caddy pour l'API). Sans --full, seul Kuro est installé (et Docker s'il manque).
#
#   sudo bash install.sh api --public-host panel.example.com [--full]
#   KURO_ENROLL_TOKEN=… sudo -E bash install.sh node --api https://panel.example.com:8443 \
#       --ca-fingerprint sha256:… [--ports 25565-25600] [--full]
#   sudo bash install.sh api node --public-host panel.example.com --ports 25565-25600 --full
#
# - Chaque archive est vérifiée avant installation : signature minisign de `SHA256SUMS`, puis somme
#   de l'archive.
# - Le token d'enrôlement passe par `KURO_ENROLL_TOKEN` ou `--token-file`, jamais en argument.
# - Relancé, le script met à jour les binaires, le panel et les prefabs, sans toucher aux secrets,
#   à la configuration, à la PKI ni à un nœud déjà enrôlé.
# - `--plan` affiche ce qui serait fait, sans rien changer.
set -euo pipefail

# Rôle pré-choisi par les raccourcis get.kuroctl.com/api et /node (réécrit à la publication).
DEFAULT_ROLES=""

KURO_REPO="${KURO_REPO:-kuroctl/kuro}"
# Releases de ce dépôt ; un miroir peut la remplacer (la signature est vérifiée quand même).
KURO_RELEASE_URL="${KURO_RELEASE_URL:-https://github.com/$KURO_REPO/releases/download}"
# Clé publique minisign des releases (la clé privée est un secret GitHub, ADR-0016). Un fork qui
# laisse un gabarit `…REPLACE…` voit l'installation refusée plutôt que non vérifiée.
MINISIGN_PUBKEY="${KURO_MINISIGN_PUBKEY:-RWRtWneTRrp1ruOA8XasvWbs6Uch5gWhK3EzHnwQPsbREM/DKJvnWcTY}"
# Clé du dépôt APT officiel de Docker (https://docs.docker.com/engine/install/debian/).
DOCKER_GPG_FINGERPRINT=9DC858229FC7DD38854AE2D88D81803C0EBFCD88
POSTGRES_IMAGE=docker.io/library/postgres:17.6

CONFIG_DIR=/etc/kuro
API_CONFIG="$CONFIG_DIR/api.toml"
SECRETS="$CONFIG_DIR/api.env"
PKI_DIR="$CONFIG_DIR/pki"
DAEMON_CONFIG="$CONFIG_DIR/daemon.toml"
DATA_DIR=/var/lib/kuro
SHARE_DIR=/usr/share/kuro
BIN_DIR=/usr/local/bin
ADMIN_TOOL=/usr/local/sbin/kuro-api-admin
API_UNIT=/etc/systemd/system/kuro-api.service
DAEMON_UNIT=/etc/systemd/system/kuro-daemon.service
SFTP_UNIT=/etc/systemd/system/kuro-sftp.service
SFTP_HOST_KEY="$CONFIG_DIR/sftp_host_ed25519"
SSH_DROPIN=/etc/ssh/sshd_config.d/50-kuro.conf

roles=()
full=false
plan=false
assume_yes=false
public_host=""
database_url=""
http_bind=127.0.0.1:8080
daemon_port=8443
api=""
fingerprint=""
token_file=""
ports=""
sftp_port=2022
sftp=true
version="${KURO_VERSION:-latest}"

usage() {
  cat <<'EOF'
Usage: install.sh [api] [node] [options]

Roles (one or both):
  api   Kuro API, panel and official prefabs
  node  Kuro daemon (runs game servers with Docker)

Preparation:
  --full            Prepare a fresh server first: updates, automatic security updates, firewall
                    (ufw), SSH key-only, fail2ban, Docker, and Caddy for the api role
                    (Debian 12-13, Ubuntu 22.04-24.04)
  --plan            Show what would be done, change nothing
  --yes             Do not ask for confirmation (--full)

api:
  --public-host     Public name (or IP) of the API: daemon listener certificate, enrollment URL
  --database-url    Existing PostgreSQL (otherwise: local `kuro-postgres` container)
  --http-bind       Local HTTP listener, behind the reverse proxy (default 127.0.0.1:8080)
  --daemon-port     Daemon listener port, mutual TLS (default 8443)

node:
  --api             API daemon listener (https://host:8443), required on first enrollment
  --ca-fingerprint  CA fingerprint shown when the node was added, required on first enrollment
  --token-file      File holding the enrollment token (otherwise: KURO_ENROLL_TOKEN)
  --ports           Port range of the game servers, opened by --full (e.g. 25565-25600)
  --sftp-port       SFTP port of the node (default 2022), opened by --full
  --no-sftp         Do not serve SFTP on this node

  --version         Version to install (default: latest stable)
EOF
}

fail() {
  echo "error: $*" >&2
  exit 1
}

step() {
  echo "==> $*"
}

while [ $# -gt 0 ]; do
  case "$1" in
    api | node) roles+=("$1"); shift ;;
    --full) full=true; shift ;;
    --plan) plan=true; shift ;;
    --yes | -y) assume_yes=true; shift ;;
    --public-host) public_host="${2:-}"; shift 2 ;;
    --database-url) database_url="${2:-}"; shift 2 ;;
    --http-bind) http_bind="${2:-}"; shift 2 ;;
    --daemon-port) daemon_port="${2:-}"; shift 2 ;;
    --api) api="${2:-}"; shift 2 ;;
    --ca-fingerprint) fingerprint="${2:-}"; shift 2 ;;
    --token-file) token_file="${2:-}"; shift 2 ;;
    --ports) ports="${2:-}"; shift 2 ;;
    --sftp-port) sftp_port="${2:-}"; shift 2 ;;
    --no-sftp) sftp=false; shift ;;
    --version) version="${2:-}"; shift 2 ;;
    --token) fail "--token is not supported: use KURO_ENROLL_TOKEN or --token-file (kept out of the shell history)" ;;
    -h | --help) usage; exit 0 ;;
    *) usage >&2; fail "unknown option: $1" ;;
  esac
done

if [ ${#roles[@]} -eq 0 ] && [ -n "$DEFAULT_ROLES" ]; then
  read -r -a roles <<<"$DEFAULT_ROLES"
fi
[ ${#roles[@]} -gt 0 ] || { usage >&2; fail "choose a role: api, node, or both"; }
if ! [[ "$sftp_port" =~ ^[0-9]{1,5}$ ]] || [ "$sftp_port" -lt 1 ] || [ "$sftp_port" -gt 65535 ]; then
  fail "--sftp-port expects a port, e.g. 2022"
fi
if [ -n "$ports" ] && ! [[ "$ports" =~ ^[0-9]{1,5}(-[0-9]{1,5})?$ ]]; then
  fail "--ports expects a port or a range, e.g. 25565-25600"
fi

has_role() {
  local role
  for role in "${roles[@]}"; do [ "$role" = "$1" ] && return 0; done
  return 1
}

# --- Prérequis ---------------------------------------------------------------------------------

[ "$(uname -s)" = Linux ] || fail "Linux only"
case "$(uname -m)" in
  x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
  aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
  *) fail "unsupported architecture: $(uname -m)" ;;
esac
$plan || [ "$(id -u)" -eq 0 ] || fail "run as root (sudo -E keeps KURO_ENROLL_TOKEN)"
$plan || command -v systemctl >/dev/null || fail "systemd is required"
$plan || command -v curl >/dev/null || fail "curl is required"
case "$MINISIGN_PUBKEY" in
  *REPLACE*) fail "this script has no release public key yet: refusing to install unverified binaries" ;;
esac

os_id=""
os_version=""
os_codename=""
if [ -r /etc/os-release ]; then
  # shellcheck disable=SC1091
  . /etc/os-release
  os_id="${ID:-}"
  os_version="${VERSION_ID:-}"
  os_codename="${VERSION_CODENAME:-}"
fi
supported_os() {
  case "$os_id:$os_version" in
    debian:12 | debian:13 | ubuntu:22.04 | ubuntu:24.04) return 0 ;;
  esac
  return 1
}

if $full && ! supported_os; then
  fail "--full supports Debian 12-13 and Ubuntu 22.04-24.04 (found ${os_id:-unknown} ${os_version:-}); install without --full"
fi
api_first_install=false
if has_role api; then
  [ -f "$API_CONFIG" ] || api_first_install=true
  if $api_first_install && [ -z "$public_host" ]; then
    fail "first api install: --public-host is required"
  fi
fi

# Nœud à enrôler : sur la même machine que l'API, le script crée le nœud lui-même.
node_enroll=false
node_local=false
if has_role node && [ ! -f "$DAEMON_CONFIG" ]; then
  node_enroll=true
  if has_role api && [ -z "$api" ] && [ -z "${KURO_ENROLL_TOKEN:-}" ] && [ -z "$token_file" ]; then
    node_local=true
    # Le daemon joint l'API par ce nom, que porte son certificat : il doit déjà résoudre.
    host="${public_host:-}"
    [ -n "$host" ] || host="$(sed -n 's|^public_url = "https://\(.*\)"|\1|p' "$API_CONFIG" 2>/dev/null)"
    [ -n "$host" ] || fail "api and node on one machine: --public-host is required"
    public_host="$host"
    getent hosts "$public_host" >/dev/null ||
      fail "$public_host does not resolve yet: point it to this machine, then rerun"
  else
    if [ -z "$api" ] || [ -z "$fingerprint" ]; then
      fail "first node install: --api and --ca-fingerprint are required"
    fi
    if [ -n "$token_file" ]; then
      [ -r "$token_file" ] || fail "cannot read $token_file"
    elif [ -z "${KURO_ENROLL_TOKEN:-}" ]; then
      fail "first node install: set KURO_ENROLL_TOKEN or pass --token-file"
    fi
  fi
fi

docker_needed=false
if has_role node; then docker_needed=true; fi
if has_role api && $api_first_install && [ -z "$database_url" ]; then docker_needed=true; fi
docker_present=false
docker_installed_now=false
docker info >/dev/null 2>&1 && docker_present=true

ssh_port() {
  local port=""
  if command -v sshd >/dev/null; then
    port="$(sshd -T 2>/dev/null | awk '$1 == "port" { print $2; exit }')"
  fi
  echo "${port:-22}"
}

# Une clé SSH est autorisée pour root ou pour l'utilisateur qui a lancé sudo.
ssh_key_present() {
  local home
  [ -s /root/.ssh/authorized_keys ] && return 0
  if [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != root ]; then
    home="$(getent passwd "$SUDO_USER" | cut -d: -f6)"
    [ -n "$home" ] && [ -s "$home/.ssh/authorized_keys" ] && return 0
  fi
  return 1
}

# --- Plan ----------------------------------------------------------------------------------------

print_plan() {
  echo "Kuro install plan (roles: ${roles[*]}):"
  if $full; then
    echo "  system   update packages, enable automatic security updates (unattended-upgrades)"
    echo "  system   install fail2ban"
    echo "  firewall ufw: deny incoming, allow SSH on port $(ssh_port) (rate limited)"
    if has_role api; then
      echo "  firewall ufw: allow 80/tcp, 443/tcp (Caddy) and $daemon_port/tcp (daemons)"
    elif has_role node; then
      echo "  firewall ufw: allow 80/tcp, 443/tcp (web sites proxy)"
    fi
    if has_role node && $sftp; then
      echo "  firewall ufw: allow $sftp_port/tcp (SFTP)"
    fi
    if has_role node; then
      if [ -n "$ports" ]; then
        echo "  firewall ufw: allow game ports $ports (tcp and udp)"
      else
        echo "  firewall no game port range given (--ports): open it later with ufw"
      fi
    fi
    if ssh_key_present; then
      echo "  ssh      key-only login, root by key only ($SSH_DROPIN)"
    else
      echo "  ssh      unchanged: no authorized key found for root or ${SUDO_USER:-the sudo user}"
    fi
    if has_role api; then
      echo "  caddy    install, publish https://${public_host:-<public host>} -> $http_bind"
    fi
  fi
  if $docker_needed && ! $docker_present; then
    echo "  docker   install from the official Docker repository (signed)"
  fi
  if has_role api; then
    if $api_first_install; then
      echo "  api      install kuro-api, panel and prefabs; config, secrets, private CA"
      [ -z "$database_url" ] && echo "  api      start a local PostgreSQL container (kuro-postgres)"
    else
      echo "  api      update kuro-api, panel and prefabs (config, secrets and CA kept)"
    fi
  fi
  if has_role node; then
    if $node_local; then
      echo "  node     install kuro-daemon, add this machine as a node of the local API and enroll it"
    elif $node_enroll; then
      echo "  node     install kuro-daemon and enroll it with $api"
    else
      echo "  node     update kuro-daemon (already enrolled)"
    fi
  fi
}

if $plan; then
  print_plan
  exit 0
fi

if $full && ! $assume_yes; then
  print_plan
  echo
  # Le script peut arriver par un tube (curl | bash) : la réponse se lit sur le terminal.
  if [ -r /dev/tty ]; then
    read -r -p "Apply these changes? [y/N] " answer </dev/tty
    case "$answer" in y | Y | yes | oui) ;; *) fail "aborted" ;; esac
  else
    fail "--full needs a confirmation: rerun with --yes"
  fi
fi

# --- Préparation du système (--full) -------------------------------------------------------------

export DEBIAN_FRONTEND=noninteractive

apt_install() {
  apt-get install -y -qq "$@" >/dev/null
}

prepare_system() {
  step "updating the system"
  apt-get update -qq
  apt-get upgrade -y -qq >/dev/null
  step "enabling automatic security updates"
  apt_install unattended-upgrades
  cat >/etc/apt/apt.conf.d/20auto-upgrades <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
EOF
  step "installing fail2ban"
  apt_install fail2ban
  systemctl enable --quiet --now fail2ban
}

configure_firewall() {
  local ssh
  ssh="$(ssh_port)"
  step "configuring the firewall (ufw)"
  apt_install ufw
  ufw default deny incoming >/dev/null
  ufw default allow outgoing >/dev/null
  # Le port SSH réellement utilisé, avant toute activation : la session en cours reste ouverte.
  ufw limit "$ssh/tcp" >/dev/null
  if has_role api; then
    ufw allow 80/tcp >/dev/null
    ufw allow 443/tcp >/dev/null
    ufw allow "$daemon_port/tcp" >/dev/null
  fi
  if has_role node; then
    # Proxy des sites web du nœud (ADR-0038) : HTTPS et défis ACME.
    ufw allow 80/tcp >/dev/null
    ufw allow 443/tcp >/dev/null
  fi
  if has_role node && $sftp; then
    # SFTP des fichiers des serveurs (ADR-0047).
    ufw allow "$sftp_port/tcp" >/dev/null
  fi
  if has_role node && [ -n "$ports" ]; then
    ufw allow "${ports/-/:}/tcp" >/dev/null
    ufw allow "${ports/-/:}/udp" >/dev/null
  fi
  ufw --force enable >/dev/null
}

# Réglages de Docker pour un nœud qui sert des sites web (ADR-0038) : des réseaux par serveur en
# nombre (les pools par défaut s'arrêtent vers trente), et sous Docker rootless le droit de lier
# les ports 80 et 443. Jamais sur un Docker déjà configuré : son `daemon.json` est à son admin.
tune_docker_for_node() {
  if $docker_installed_now && [ ! -e /etc/docker/daemon.json ]; then
    step "configuring docker address pools for per-server networks"
    install -d -m 0755 /etc/docker
    cat >/etc/docker/daemon.json <<'EOF'
{
  "default-address-pools": [
    { "base": "10.200.0.0/16", "size": 24 }
  ]
}
EOF
    systemctl restart docker
  fi
  if docker info --format '{{.SecurityOptions}}' 2>/dev/null | grep -q rootless; then
    step "allowing rootless docker to bind ports 80 and 443 (web sites proxy)"
    echo "net.ipv4.ip_unprivileged_port_start=80" >/etc/sysctl.d/90-kuro-web.conf
    sysctl -q --system >/dev/null 2>&1 || true
  fi
}

harden_ssh() {
  if ! ssh_key_present; then
    step "ssh left unchanged: no authorized key for root or ${SUDO_USER:-the sudo user}"
    return
  fi
  step "ssh: key-only login"
  install -d -m 0755 /etc/ssh/sshd_config.d
  cat >"$SSH_DROPIN" <<'EOF'
# Écrit par l'installation de Kuro (--full) : connexion par clé seulement.
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password
EOF
  if sshd -t 2>/dev/null; then
    systemctl reload ssh 2>/dev/null || systemctl reload sshd
  else
    rm -f "$SSH_DROPIN"
    step "ssh configuration rejected by sshd -t: left unchanged"
  fi
}

install_docker() {
  $docker_present && return
  supported_os ||
    fail "Docker is not running. Install it from your distribution or https://docs.docker.com/engine/install/, then rerun."
  step "installing Docker from the official repository"
  apt-get update -qq
  apt_install ca-certificates curl gnupg
  install -d -m 0755 /etc/apt/keyrings
  curl -fsSL "https://download.docker.com/linux/$os_id/gpg" -o /etc/apt/keyrings/docker.asc
  local found
  found="$(gpg --show-keys --with-colons /etc/apt/keyrings/docker.asc 2>/dev/null |
    awk -F: '$1 == "fpr" { print $10; exit }')"
  if [ "$found" != "$DOCKER_GPG_FINGERPRINT" ]; then
    rm -f /etc/apt/keyrings/docker.asc
    fail "the Docker repository key does not match the expected fingerprint: refusing to install"
  fi
  chmod a+r /etc/apt/keyrings/docker.asc
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/$os_id $os_codename stable" \
    >/etc/apt/sources.list.d/docker.list
  apt-get update -qq
  apt_install docker-ce docker-ce-cli containerd.io
  systemctl enable --quiet --now docker
  docker info >/dev/null 2>&1 || fail "Docker did not start: journalctl -u docker"
  docker_present=true
  docker_installed_now=true
}

install_caddy() {
  step "installing Caddy (TLS for $public_host)"
  apt_install caddy
  local caddyfile=/etc/caddy/Caddyfile
  local site
  site="$(printf '%s {\n\treverse_proxy %s\n}\n' "$public_host" "$http_bind")"
  if [ -f "$caddyfile" ] && grep -q "^$public_host" "$caddyfile"; then
    return
  fi
  # Le Caddyfile d'origine du paquet sert une page d'accueil sur :80 : il est remplacé. Un
  # Caddyfile déjà modifié est gardé, et le bloc à ajouter est affiché.
  if [ ! -f "$caddyfile" ] || grep -q '^:80' "$caddyfile"; then
    printf '%s' "$site" >"$caddyfile"
    systemctl reload caddy 2>/dev/null || systemctl restart caddy
  else
    step "existing $caddyfile kept: add this block, then reload caddy"
    printf '%s' "$site"
  fi
}

# --- Téléchargement vérifié ----------------------------------------------------------------------

install_minisign() {
  command -v minisign >/dev/null && return
  step "installing minisign"
  if command -v apt-get >/dev/null; then
    apt-get update -qq && apt_install minisign
  elif command -v dnf >/dev/null; then
    dnf install -y -q minisign
  elif command -v apk >/dev/null; then
    apk add --quiet minisign
  else
    fail "install minisign with your package manager, then rerun"
  fi
}

random_hex() {
  head -c "$1" /dev/urandom | od -An -tx1 | tr -d ' \n'
}

resolve_version() {
  if [ "$version" = latest ]; then
    version="$(curl -fsSL "https://api.github.com/repos/$KURO_REPO/releases/latest" |
      sed -n 's/.*"tag_name": *"v\([^"]*\)".*/\1/p' | head -n 1)"
    [ -n "$version" ] || fail "cannot find the latest release of $KURO_REPO"
  fi
  version="${version#v}"
}

# Télécharge l'archive et la vérifie ; affiche le dossier extrait.
fetch() {
  local name="$1" work="$2" base archive
  base="$KURO_RELEASE_URL/v$version"
  archive="$name-$version-$target.tar.gz"
  curl -fsSL -o "$work/SHA256SUMS" "$base/SHA256SUMS"
  curl -fsSL -o "$work/SHA256SUMS.minisig" "$base/SHA256SUMS.minisig"
  curl -fsSL -o "$work/$archive" "$base/$archive"
  minisign -Vq -P "$MINISIGN_PUBKEY" -m "$work/SHA256SUMS" ||
    fail "SHA256SUMS signature is invalid: refusing to install"
  (cd "$work" && grep -F " $archive" SHA256SUMS | sha256sum -c --quiet -) ||
    fail "$archive does not match SHA256SUMS: refusing to install"
  tar -xzf "$work/$archive" -C "$work"
  echo "$work/$name-$version-$target"
}

# --- Rôle api ------------------------------------------------------------------------------------

install_api() {
  step "downloading kuro-api $version ($target)"
  local extracted
  extracted="$(fetch kuro-api "$work")"
  install -m 0755 "$extracted/kuro-api" "$BIN_DIR/"
  install -d -m 0755 "$SHARE_DIR"
  local part
  for part in panel prefabs; do
    rm -rf "$SHARE_DIR/$part.new"
    cp -r "$extracted/$part" "$SHARE_DIR/$part.new"
    rm -rf "${SHARE_DIR:?}/${part:?}"
    mv "$SHARE_DIR/$part.new" "$SHARE_DIR/$part"
  done

  if ! id kuro >/dev/null 2>&1; then
    step "creating the kuro system user"
    useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin kuro
  fi
  install -d -m 0750 -o root -g kuro "$CONFIG_DIR"
  install -d -m 0750 -o kuro -g kuro "$DATA_DIR" "$PKI_DIR"

  if [ ! -f "$SECRETS" ]; then
    if [ -z "$database_url" ]; then
      docker info >/dev/null 2>&1 ||
        fail "no --database-url and Docker is not running: install Docker or provide PostgreSQL"
      step "starting a local PostgreSQL (kuro-postgres)"
      local db_password
      db_password="$(random_hex 24)"
      docker run -d --name kuro-postgres --restart unless-stopped \
        -p 127.0.0.1:5432:5432 -v kuro-postgres-data:/var/lib/postgresql/data \
        -e POSTGRES_USER=kuro -e POSTGRES_DB=kuro -e POSTGRES_PASSWORD="$db_password" \
        "$POSTGRES_IMAGE" >/dev/null
      database_url="postgres://kuro:$db_password@127.0.0.1:5432/kuro"
      # La PKI s'initialise avec son événement d'audit : la base doit répondre.
      step "waiting for PostgreSQL"
      local _
      for _ in $(seq 1 60); do
        docker exec kuro-postgres pg_isready -q -U kuro -d kuro && break
        sleep 1
      done
      docker exec kuro-postgres pg_isready -q -U kuro -d kuro ||
        fail "PostgreSQL did not start: docker logs kuro-postgres"
    fi
    step "writing secrets to $SECRETS"
    umask 077
    cat >"$SECRETS" <<EOF
# Secrets de l'API Kuro : jamais dans api.toml, jamais affichés.
KURO_API_DATABASE__URL=$database_url
KURO_API_AUTH__SECRET_KEY=$(random_hex 32)
EOF
    umask 022
    chown root:kuro "$SECRETS"
    chmod 0640 "$SECRETS"
  fi

  if $api_first_install; then
    step "writing $API_CONFIG"
    cat >"$API_CONFIG" <<EOF
# Configuration de l'API Kuro. Les secrets sont dans api.env (variables KURO_API_*).

[http]
bind = "$http_bind"
panel_dir = "$SHARE_DIR/panel"

[daemon]
bind = "0.0.0.0:$daemon_port"
ca_cert = "$PKI_DIR/ca.pem"
ca_key = "$PKI_DIR/ca.key"
cert = "$PKI_DIR/api.pem"
key = "$PKI_DIR/api.key"

[auth]
require_totp = true

[prefabs]
dir = "$SHARE_DIR/prefabs"

[mcp]
# Serveur MCP des agents IA (ADR-0023), ouvert ou coupé dans Paramètres › Projet.
public_url = "https://$public_host"

[log]
format = "json"
filter = "info"
EOF
    chmod 0640 "$API_CONFIG"
    chown root:kuro "$API_CONFIG"
  fi

  # Commandes d'administration avec la configuration et les secrets de l'API, en tant que `kuro`.
  cat >"$ADMIN_TOOL" <<EOF
#!/bin/sh
# Lance kuro-api avec sa configuration (ex. : kuro-api-admin admin create --email …).
set -a
. $SECRETS
set +a
exec runuser -u kuro -- $BIN_DIR/kuro-api --config $API_CONFIG "\$@"
EOF
  chmod 0750 "$ADMIN_TOOL"

  if [ ! -f "$PKI_DIR/ca.pem" ]; then
    # Applique aussi les migrations : la création de la CA est auditée.
    step "creating the private CA for $public_host"
    "$ADMIN_TOOL" pki init --name "$public_host"
  fi

  step "installing the kuro-api systemd unit"
  cat >"$API_UNIT" <<EOF
[Unit]
Description=Kuro API
Documentation=https://github.com/$KURO_REPO
After=network-online.target docker.service
Wants=network-online.target

[Service]
User=kuro
Group=kuro
EnvironmentFile=$SECRETS
ExecStart=$BIN_DIR/kuro-api --config $API_CONFIG serve
Restart=on-failure
RestartSec=5
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
RestrictSUIDSGID=yes
LockPersonality=yes
CapabilityBoundingSet=
ReadWritePaths=$DATA_DIR

[Install]
WantedBy=multi-user.target
EOF
  systemctl daemon-reload
  systemctl enable --quiet kuro-api
  systemctl restart kuro-api
  step "kuro-api $version is running: journalctl -u kuro-api -f"
}

# --- Rôle node -----------------------------------------------------------------------------------

# Nœud de l'API installée sur cette machine : il est créé puis enrôlé sans afficher le jeton.
enroll_local_node() {
  local name ticket token fp
  name="$(hostname -s | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9-\n' '-' | cut -c1-63)"
  [ -n "$name" ] || name=node
  step "adding this machine as node $name"
  local _
  for _ in $(seq 1 30); do
    ticket="$("$ADMIN_TOOL" node add --name "$name" 2>/dev/null)" && break
    sleep 1
  done
  [ -n "${ticket:-}" ] || fail "cannot add node $name: kuro-api-admin node add --name $name"
  token="$(awk 'found { print $1; exit } /^enrollment token/ { found = 1 }' <<<"$ticket")"
  fp="$(grep -o 'sha256:[0-9a-fA-F:]*' <<<"$ticket" | head -n 1)"
  if [ -z "$token" ] || [ -z "$fp" ]; then
    fail "cannot read the enrollment ticket of node $name"
  fi
  KURO_ENROLL_TOKEN="$token" "$BIN_DIR/kuro-daemonctl" --config "$DAEMON_CONFIG" enroll \
    --api "https://$public_host:$daemon_port" --ca-fingerprint "$fp"
}

install_node() {
  step "downloading kuro-daemon $version ($target)"
  local extracted
  extracted="$(fetch kuro-daemon "$work")"
  install -m 0755 "$extracted/kuro-daemon" "$extracted/kuro-daemonctl" "$BIN_DIR/"

  install -d -m 0711 "$DATA_DIR"
  # Le dossier de configuration est partagé avec l'API sur une machine qui a les deux rôles.
  if [ ! -d "$CONFIG_DIR" ]; then
    install -d -m 0700 "$CONFIG_DIR"
  fi

  if ! $node_enroll; then
    step "node already enrolled ($DAEMON_CONFIG kept)"
  elif $node_local; then
    enroll_local_node
  else
    step "enrolling this node"
    local enroll=("$BIN_DIR/kuro-daemonctl" --config "$DAEMON_CONFIG" enroll --api "$api" --ca-fingerprint "$fingerprint")
    [ -n "$token_file" ] && enroll+=(--token-file "$token_file")
    "${enroll[@]}"
  fi

  # SFTP (ADR-0047) : le daemon l'annonce et tient le socket, `kuro-sftp` sert le SSH.
  local sftp_env=""
  if $sftp; then
    sftp_env="Environment=KURO_DAEMON_SFTP__BIND=0.0.0.0:$sftp_port"
  fi

  step "installing the kuro-daemon systemd unit"
  cat >"$DAEMON_UNIT" <<EOF
[Unit]
Description=Kuro daemon
Documentation=https://github.com/$KURO_REPO
After=network-online.target docker.service
Wants=network-online.target
Requires=docker.service

[Service]
ExecStart=$BIN_DIR/kuro-daemon --config $DAEMON_CONFIG
$sftp_env
Restart=on-failure
RestartSec=5
# Root : socket Docker et attribution des fichiers des serveurs à leur utilisateur (ADR-0015).
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
ReadWritePaths=$DATA_DIR $CONFIG_DIR
# Socket du processus SFTP (/run/kuro/sftp.sock), gardé entre deux redémarrages.
RuntimeDirectory=kuro
RuntimeDirectoryMode=0755
RuntimeDirectoryPreserve=yes

[Install]
WantedBy=multi-user.target
EOF
  systemctl daemon-reload
  systemctl enable --quiet kuro-daemon
  systemctl restart kuro-daemon
  step "kuro-daemon $version is running: journalctl -u kuro-daemon -f"
  if $sftp; then
    install_sftp
  elif [ -f "$SFTP_UNIT" ]; then
    systemctl disable --now --quiet kuro-sftp || true
    rm -f "$SFTP_UNIT"
    systemctl daemon-reload
  fi
}

# Processus SFTP (ADR-0047) : utilisateur dédié, hors du groupe docker, qui ne voit que les
# données des serveurs, sa clé d'hôte (transmise par systemd) et le socket du daemon.
install_sftp() {
  if ! id -u kuro-sftp >/dev/null 2>&1; then
    useradd --system --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin kuro-sftp
  fi
  install -d -m 0711 "$DATA_DIR/servers"
  # La clé d'hôte est créée par le daemon à son démarrage.
  for _ in $(seq 1 15); do
    [ -s "$SFTP_HOST_KEY" ] && break
    sleep 1
  done
  [ -s "$SFTP_HOST_KEY" ] || fail "kuro-daemon did not create the SFTP host key ($SFTP_HOST_KEY)"

  step "installing the kuro-sftp systemd unit"
  cat >"$SFTP_UNIT" <<EOF
[Unit]
Description=Kuro SFTP
Documentation=https://github.com/$KURO_REPO
After=network-online.target kuro-daemon.service
Wants=network-online.target

[Service]
User=kuro-sftp
Group=kuro-sftp
ExecStart=$BIN_DIR/kuro-daemon --config $DAEMON_CONFIG sftp
Environment=KURO_DAEMON_SFTP__BIND=0.0.0.0:$sftp_port
Restart=on-failure
RestartSec=5
LoadCredential=host-key:$SFTP_HOST_KEY
# Écrire dans des volumes qui appartiennent aux utilisateurs des services, rien d'autre.
AmbientCapabilities=CAP_CHOWN CAP_FOWNER CAP_DAC_OVERRIDE
CapabilityBoundingSet=CAP_CHOWN CAP_FOWNER CAP_DAC_OVERRIDE
NoNewPrivileges=yes
TemporaryFileSystem=/:ro
BindReadOnlyPaths=$BIN_DIR/kuro-daemon $DAEMON_CONFIG
BindPaths=$DATA_DIR/servers /run/kuro
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
RestrictNamespaces=yes
RestrictSUIDSGID=yes
RestrictRealtime=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
SystemCallFilter=@system-service
MemoryMax=512M
TasksMax=512

[Install]
WantedBy=multi-user.target
EOF
  systemctl daemon-reload
  systemctl enable --quiet kuro-sftp
  systemctl restart kuro-sftp
  step "kuro-sftp is listening on port $sftp_port: journalctl -u kuro-sftp -f"
}

# --- Déroulé -------------------------------------------------------------------------------------

if $full; then
  prepare_system
  configure_firewall
  harden_ssh
fi
if $docker_needed; then
  install_docker
fi
install_minisign
resolve_version
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

if has_role api; then
  install_api
  if $full; then
    install_caddy
  fi
fi
if has_role node; then
  tune_docker_for_node
  install_node
fi

if has_role api && $api_first_install; then
  next=2
  echo
  echo "Next steps:"
  if ! $full; then
    cat <<EOF
  1. Publish the panel over TLS, for example with Caddy (/etc/caddy/Caddyfile):
       $public_host {
           reverse_proxy $http_bind
       }
     and open port $daemon_port/tcp to your nodes (daemon link, mutual TLS).
EOF
  else
    echo "  1. Point $public_host to this machine: Caddy obtains the certificate on the first visit."
  fi
  # Première mise en route (ADR-0045) : l'API écrit le jeton au démarrage, tant qu'aucun compte
  # n'existe. Lu ici pour l'afficher à qui a lancé l'installation, jamais journalisé ailleurs.
  setup_token=""
  for _ in $(seq 1 15); do
    [ -s "$DATA_DIR/setup-token" ] && setup_token="$(cat "$DATA_DIR/setup-token")" && break
    sleep 1
  done
  if [ -n "$setup_token" ]; then
    echo "  $next. Open https://$public_host and follow the setup: your setup token is"
    echo "       $setup_token"
  else
    echo "  $next. Open https://$public_host and follow the setup with the token in $DATA_DIR/setup-token"
    echo "       (written once kuro-api runs: sudo cat $DATA_DIR/setup-token)."
  fi
  echo "     Without the panel: sudo kuro-api-admin setup --email you@example.com --name \"Your Name\""
  if ! has_role node; then
    echo "  $((next + 1)). Add a node in the panel, then on it: curl -fsSL https://get.kuroctl.com/node | sudo -E bash -s -- --api https://$public_host:$daemon_port --ca-fingerprint …"
  fi
fi
