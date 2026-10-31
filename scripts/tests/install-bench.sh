#!/usr/bin/env bash
# Banc de test du script d'installation (ADR-0028), lancé par install-test.sh dans un conteneur Debian :
# fausse release signée servie en local, systemctl et docker simulés.
# Monté : /bin-musl (binaires musl), /panel (panel compilé), /src (dépôt).
set -euo pipefail

pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*"; exit 1; }
expect_failure() {
  local label="$1" pattern="$2"; shift 2
  local out
  if out="$("$@" 2>&1)"; then fail "$label: succeeded"; fi
  grep -q -- "$pattern" <<<"$out" || { echo "$out"; fail "$label: missing '$pattern'"; }
  pass "$label"
}

apt-get update -qq >/dev/null
apt-get install -y -qq minisign python3 curl ca-certificates >/dev/null

# La fausse release porte la version du binaire (Cargo.toml) : install.sh la relit par `--version`.
VERSION="$(/bin-musl/kuro-api --version | awk '{print $2}')"
[ -n "$VERSION" ] || fail "cannot read the version of /bin-musl/kuro-api"
T=x86_64-unknown-linux-musl
R=/release/v$VERSION
mkdir -p "$R" /work
cd /work
mkdir "kuro-api-$VERSION-$T" "kuro-daemon-$VERSION-$T"
install -m 0755 /bin-musl/kuro-api "kuro-api-$VERSION-$T/"
cp -r /panel "kuro-api-$VERSION-$T/panel"
cp -r /src/prefabs "kuro-api-$VERSION-$T/prefabs"
install -m 0755 /bin-musl/kuro-daemon /bin-musl/kuro-daemonctl "kuro-daemon-$VERSION-$T/"
tar -czf "$R/kuro-api-$VERSION-$T.tar.gz" "kuro-api-$VERSION-$T"
tar -czf "$R/kuro-daemon-$VERSION-$T.tar.gz" "kuro-daemon-$VERSION-$T"
(cd "$R" && sha256sum -- *.tar.gz > SHA256SUMS)
minisign -G -W -p /work/kuro.pub -s /work/kuro.key >/dev/null
minisign -S -s /work/kuro.key -m "$R/SHA256SUMS" >/dev/null
minisign -G -W -p /work/other.pub -s /work/other.key >/dev/null
PUB="$(tail -n 1 /work/kuro.pub)"
OTHER="$(tail -n 1 /work/other.pub)"

(cd /release && python3 -m http.server 8000 >/dev/null 2>&1 &)
sleep 1

# systemd et Docker simulés : on vérifie ce que le script demande.
cat >/usr/local/bin/systemctl <<'EOF'
#!/bin/sh
echo "systemctl $*" >> /tmp/systemctl.log
EOF
cat >/usr/local/bin/docker <<'EOF'
#!/bin/sh
[ "$1" = info ] && exit 0
echo "docker $*" >> /tmp/docker.log
EOF
chmod +x /usr/local/bin/systemctl /usr/local/bin/docker

export KURO_RELEASE_URL=http://127.0.0.1:8000
API=(bash /src/scripts/install.sh api --public-host api.example.test --database-url "$KURO_TEST_DATABASE_URL" --version "$VERSION")
DAEMON=(bash /src/scripts/install.sh node --version "$VERSION")

# --- Plan, rôles et raccourcis ----------------------------------------------------------------------
expect_failure "a role is required" "choose a role" bash /src/scripts/install.sh --version "$VERSION"
expect_failure "unknown option refused" "unknown option" bash /src/scripts/install.sh api --nope
expect_failure "bad port range refused" "expects a port" bash /src/scripts/install.sh node --ports 25565:25600
out="$(bash /src/scripts/install.sh api node --public-host localhost --ports 25565-25600 --full --plan)"
for line in "ufw: deny incoming, allow SSH on port 22" "allow 80/tcp, 443/tcp (Caddy) and 8443/tcp" \
  "allow game ports 25565-25600" "ssh      unchanged: no authorized key" "caddy    install" \
  "add this machine as a node of the local API"; do
  grep -qF -- "$line" <<<"$out" || { echo "$out"; fail "plan misses: $line"; }
done
if [ -e /etc/kuro ] || [ -e /usr/local/bin/kuro-api ]; then fail "--plan changed the system"; fi
pass "--plan shows the full preparation and changes nothing"
# Raccourci get.kuroctl.com/node : rôle pré-choisi, réécrit à la publication.
sed 's/^DEFAULT_ROLES=""/DEFAULT_ROLES="node"/' /src/scripts/install.sh >/work/node.sh
KURO_ENROLL_TOKEN=x /bin/bash /work/node.sh --plan --api https://api.example.test:8443 --ca-fingerprint sha256:00 |
  grep -q "enroll it with https://api.example.test:8443" || fail "node shortcut"
pass "node shortcut picks its role"

# --- Refus ---------------------------------------------------------------------------------------
expect_failure "placeholder key refused" "no release public key" env KURO_MINISIGN_PUBKEY=RWREPLACE "${API[@]}"
# La clé officielle inscrite dans le script refuse une release signée par une autre clé.
expect_failure "official key refuses a foreign release" "signature is invalid" env -u KURO_MINISIGN_PUBKEY "${API[@]}"
expect_failure "wrong key refused" "signature is invalid" env KURO_MINISIGN_PUBKEY="$OTHER" "${API[@]}"
[ ! -e /usr/local/bin/kuro-api ] || fail "binary installed despite a bad signature"
pass "nothing installed after refusals"

# --- Rôle api --------------------------------------------------------------------------------------
KURO_MINISIGN_PUBKEY="$PUB" "${API[@]}" >/tmp/api.out
/usr/local/bin/kuro-api --version | grep -q "$VERSION" || fail "kuro-api not installed"
[ -f /usr/share/kuro/panel/index.html ] || fail "panel not installed"
[ -f /usr/share/kuro/prefabs/minecraft-java/prefab.yaml ] || fail "prefabs not installed"
grep -q 'panel_dir = "/usr/share/kuro/panel"' /etc/kuro/api.toml || fail "panel_dir missing"
[ "$(stat -c '%a %U:%G' /etc/kuro/api.env)" = "640 root:kuro" ] || fail "api.env permissions: $(stat -c '%a %U:%G' /etc/kuro/api.env)"
grep -q '^KURO_API_AUTH__SECRET_KEY=[0-9a-f]\{64\}$' /etc/kuro/api.env || fail "secret key missing"
[ -f /etc/kuro/pki/ca.pem ] || fail "PKI missing"
[ "$(stat -c '%U' /etc/kuro/pki/ca.key)" = kuro ] || fail "CA key not owned by kuro"
grep -q "EnvironmentFile=/etc/kuro/api.env" /etc/systemd/system/kuro-api.service || fail "unit"
grep -q "restart kuro-api" /tmp/systemctl.log || fail "service not restarted"
grep -q "setup-token" /tmp/api.out || fail "setup hint missing"
grep -q "kuro-api-admin setup" /tmp/api.out || fail "CLI setup hint missing"
! grep -q "SECRET_KEY\|pw@" /tmp/api.out || fail "a secret was printed"
pass "api: binary, panel, prefabs, config, secrets, PKI, unit"

secret="$(grep SECRET_KEY /etc/kuro/api.env)"
ca="$(sha256sum /etc/kuro/pki/ca.pem)"
KURO_MINISIGN_PUBKEY="$PUB" "${API[@]}" >/dev/null
[ "$secret" = "$(grep SECRET_KEY /etc/kuro/api.env)" ] || fail "secret regenerated on rerun"
[ "$ca" = "$(sha256sum /etc/kuro/pki/ca.pem)" ] || fail "CA regenerated on rerun"
pass "api rerun keeps secrets and CA"

# --- Rôle node ----------------------------------------------------------------------------------
export KURO_MINISIGN_PUBKEY="$PUB"
expect_failure "token argument refused" "not supported" "${DAEMON[@]}" --token abc
expect_failure "first install needs the API" "--api and --ca-fingerprint" "${DAEMON[@]}"
expect_failure "first install needs a token" "KURO_ENROLL_TOKEN" "${DAEMON[@]}" --api https://api.example.test:8443 --ca-fingerprint sha256:00

# Nœud déjà enrôlé : mise à jour des binaires et de l'unité, sans réenrôlement.
cp /etc/kuro/pki/ca.pem /etc/kuro/ca.pem
printf '[api]\nurl = "wss://api.example.test:8443/api/daemon"\n' > /etc/kuro/daemon.toml
"${DAEMON[@]}" >/tmp/daemon.out
/usr/local/bin/kuro-daemon --version | grep -q "$VERSION" || fail "kuro-daemon not installed"
/usr/local/bin/kuro-daemonctl --version | grep -q "$VERSION" || fail "kuro-daemonctl not installed"
grep -q "already enrolled" /tmp/daemon.out || fail "re-enrolled an enrolled node"
grep -q "Requires=docker.service" /etc/systemd/system/kuro-daemon.service || fail "daemon unit"
grep -q "restart kuro-daemon" /tmp/systemctl.log || fail "daemon not restarted"
pass "node: binaries and unit, no re-enrollment"

# --- Archive altérée -------------------------------------------------------------------------------
echo tampered >> "$R/kuro-daemon-$VERSION-$T.tar.gz"
expect_failure "tampered archive refused" "does not match SHA256SUMS" "${DAEMON[@]}"

echo "ALL PASSED"
