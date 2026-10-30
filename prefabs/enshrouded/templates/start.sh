#!/bin/bash
# Lancement d'Enshrouded sous Proton GE (ADR-0032), copié sur le volume à l'installation.
#
# - Proton désactive son relais Steam (`lsteamclient`) quand `UMU_ID` est défini ; le serveur en a
#   besoin pour Steamworks. Une copie du script `proton`, sans cette ligne, est faite dans /tmp ;
#   le reste de Proton reste celui de l'image (racine en lecture seule).
# - La bibliothèque Steam (`steamclient.so`, copiée depuis SteamCMD à l'installation) est cherchée
#   dans ~/.steam/sdk64.
# - Le journal du serveur est écrit dans un fichier : il pointe sur la sortie du conteneur, que
#   Kuro relaie à la console.
# - À l'arrêt, SIGINT atteint le serveur (tini relaie au groupe) ; Proton rend la main avant la fin
#   de l'enregistrement : le script attend que le serveur soit vraiment arrêté.
set -euo pipefail

proton_dir=/tmp/proton
mkdir -p "$proton_dir" "$HOME/.steam" "$STEAM_COMPAT_DATA_PATH"
for file in /usr/local/bin/*; do
  ln -sfn "$file" "$proton_dir/"
done
rm -f "$proton_dir/proton"
sed 's/append_to_env_str(self.env, "WINEDLLOVERRIDES", "lsteamclient=d", ";")/pass/' \
  /usr/local/bin/proton > "$proton_dir/proton"
ln -sfn /data/steam/linux64 "$HOME/.steam/sdk64"
ln -sfn /data/steam/linux32 "$HOME/.steam/sdk32"

cd /data/server
mkdir -p logs savegame
ln -sf /proc/1/fd/1 logs/enshrouded_server.log

# Un gestionnaire (et non un signal ignoré, hérité par les enfants) : le shell survit au signal.
trap ':' INT TERM
python3 "$proton_dir/proton" run enshrouded_server.exe || true
while pgrep -f enshrouded_server.exe > /dev/null; do
  sleep 1
done
