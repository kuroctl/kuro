#!/bin/sh
# Point d'entrée de l'image kuro-api : au premier `serve`, crée la CA privée des nœuds dans le
# volume (nom public dans KURO_PUBLIC_HOST), puis lance kuro-api. Toute autre commande est
# passée telle quelle : `docker compose exec api kuro-entrypoint admin create --email …`.
set -eu

ca="${KURO_API_DAEMON__CA_CERT:-/var/lib/kuro/pki/ca.pem}"
if [ "${1:-serve}" = serve ] && [ ! -f "$ca" ]; then
  : "${KURO_PUBLIC_HOST:?set KURO_PUBLIC_HOST (public name of the API) to create the node CA}"
  mkdir -p "$(dirname "$ca")"
  echo "==> creating the private CA for $KURO_PUBLIC_HOST"
  kuro-api pki init --name "$KURO_PUBLIC_HOST"
fi

exec kuro-api "$@"
