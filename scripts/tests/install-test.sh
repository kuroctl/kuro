#!/usr/bin/env bash
# Teste install.sh de bout en bout (ADR-0016, ADR-0028), dans un conteneur Debian :
# vraie release signée (clé jetable), vrai PostgreSQL, systemd et Docker simulés.
#
#   scripts/tests/install-test.sh <dossier des binaires musl x86_64> <panel/dist>
#
# Prérequis : Docker. Les binaires viennent de
# `cargo zigbuild --release --target x86_64-unknown-linux-musl -p kuro-api -p kuro-daemon`.
set -euo pipefail

bins="$(cd "${1:?binaries directory}" && pwd)"
panel="$(cd "${2:?panel dist directory}" && pwd)"
repo="$(cd "$(dirname "$0")/../.." && pwd)"
network=kuro-install-test
db=kuro-install-test-db

cleanup() {
  docker rm -f "$db" >/dev/null 2>&1 || true
  docker network rm "$network" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cleanup

docker network create "$network" >/dev/null
docker run -d --name "$db" --network "$network" \
  -e POSTGRES_USER=kuro -e POSTGRES_PASSWORD=pw -e POSTGRES_DB=kuro \
  docker.io/library/postgres:17.6 >/dev/null
for _ in $(seq 1 60); do
  docker exec "$db" pg_isready -q -U kuro -d kuro 2>/dev/null && break
  sleep 1
done

# `MSYS_NO_PATHCONV` : chemins Unix intacts sous Git Bash (Windows).
MSYS_NO_PATHCONV=1 docker run --rm --network "$network" \
  -e KURO_TEST_DATABASE_URL="postgres://kuro:pw@$db/kuro" \
  -v "$bins:/bin-musl:ro" -v "$panel:/panel:ro" -v "$repo:/src:ro" \
  docker.io/library/debian:bookworm-slim bash /src/scripts/tests/install-bench.sh
