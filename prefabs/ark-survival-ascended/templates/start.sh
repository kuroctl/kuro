#!/bin/bash
# Lancement d'ARK : Survival Ascended sous Proton GE 10 (ADR-0032), copié sur le volume à
# l'installation. Les arguments du serveur (carte et options, joueurs, port, mods) arrivent en
# arguments séparés : rien n'est collé dans ce script.
#
# - Le `steamclient64.dll` livré avec le serveur le fait planter sous Proton : il est retiré.
# - Le serveur écrit son journal dans ShooterGame.log, relayé à la console de Kuro.
# - À l'arrêt, SIGINT atteint le serveur (tini relaie au groupe) ; le script attend sa fin.
set -euo pipefail

mkdir -p "$HOME" "$STEAM_COMPAT_DATA_PATH"
cd /data/server
rm -f ShooterGame/Binaries/Win64/steamclient64.dll

log=ShooterGame/Saved/Logs/ShooterGame.log
mkdir -p "$(dirname "$log")"
: > "$log"
tail -n +1 -F "$log" 2> /dev/null &

# Un gestionnaire (et non un signal ignoré, hérité par les enfants) : le shell survit au signal.
trap ':' INT TERM
proton run ./ShooterGame/Binaries/Win64/ArkAscendedServer.exe "$@" || true
while pgrep -f ArkAscendedServer.exe > /dev/null; do
  sleep 1
done
