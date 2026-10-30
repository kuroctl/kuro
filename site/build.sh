#!/usr/bin/env bash
# Construit la page d'accueil publiée sur GitHub Pages (`site/dist`) : les fichiers statiques de
# `site/public`, et les scripts d'installation servis sous /install/ (ADR-0028). Le dépôt garde
# une seule source, `scripts/install.sh` ; /install/api.sh et /install/node.sh en sont des
# copies au rôle pré-choisi. La documentation, elle, est publiée par Mintlify depuis `docs/`.
set -euo pipefail

site="$(cd "$(dirname "$0")" && pwd)"
source_script="$site/../scripts/install.sh"
marker='DEFAULT_ROLES=""'

if [ "$(grep -cx "$marker" "$source_script")" != 1 ]; then
  echo "scripts/install.sh: expected exactly one line '$marker'" >&2
  exit 1
fi

rm -rf "$site/dist"
cp -R "$site/public" "$site/dist"
mkdir -p "$site/dist/install"
for entry in "install.sh:" "api.sh:api" "node.sh:node"; do
  file="${entry%%:*}"
  roles="${entry#*:}"
  sed "s/^DEFAULT_ROLES=\"\"$/DEFAULT_ROLES=\"$roles\"/" "$source_script" > "$site/dist/install/$file"
done
# Adresses de la documentation (get.kuroctl.com/api, /node, /install.sh), sans redirection.
cp "$site/dist/install/api.sh" "$site/dist/api"
cp "$site/dist/install/node.sh" "$site/dist/node"
cp "$site/dist/install/install.sh" "$site/dist/install.sh"
echo "site built in site/dist (installers in /install/)"
