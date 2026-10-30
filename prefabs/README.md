# Prefabs Kuro

Modèles de serveurs (jeux, sites web, outils) pour [Kuro](https://github.com/kuroctl/kuro), le
panel de gestion de serveurs. Ce dépôt est inclus dans Kuro comme sous-module (`prefabs/`) :
chaque version de Kuro livre les prefabs du commit qu'elle épingle, importés au démarrage de
l'API comme prefabs officiels.

Un prefab se crée aussi depuis le **Prefab Studio** du panel (éditeur validé pendant la saisie,
essai sur un nœud de test), puis se publie dans le catalogue de l'installation. Pour le proposer à
tout le monde, ajoutez son dossier ici par une pull request.

| Prefab | Services | Points illustrés |
|---|---|---|
| `minecraft-java` | 1 | image qui s'installe seule, `server.properties` en `managed-keys`, mods et plugins en `download` (Modrinth, CurseForge, téléversement), sauvegarde à chaud (`save-off` / `save-all flush` / `save-on`, journaux exclus) |
| `wordpress` | 2 (`web`, `db`) | réseau privé entre services, secret généré partagé, healthcheck, fichiers PHP éditables |
| `terraria` | 1 | installation `download` (sha256 épinglé) + `extract` (`subdir`, `executable`) + `remove`, `serverconfig.txt` en `managed-keys` (`ini`), `HOME` sur le volume, sauvegarde après `save` |
| `ark-survival-ascended` | 1 | installation `steamcmd`, `Game.ini` libre, `GameUserSettings.ini` en `managed-keys`, mods CurseForge en `reference` (brouillon : image d'exécution à choisir) |
| `minecraft-bedrock` | 1 | image qui s'installe seule, transport RakNet (un seul port UDP), réglages par variables d'environnement, sauvegarde à chaud (`save hold` / `save query` / `save resume`), add-ons CurseForge en `extract` (packs rangés d'après leur `manifest.json`, inscrits dans `world_*_packs.json`) |
| `valheim` | 1 | `steamcmd`, image Pterodactyl gardée avec son point d'entrée (`STARTUP`, réglages par l'environnement), arrêt par SIGINT |
| `project-zomboid` | 1 | `steamcmd`, JVM livrée avec le jeu, `kuro.ini` en `managed-keys` (`ini` sans sections), emplacement Steam Workshop en `reference` (`WorkshopItems`), identifiants des mods saisis par objet (`fields`, `Mods=`), proposés d'après la description de l'objet (`prefill`) |
| `rust` | 1 | `steamcmd` puis `script` (bibliothèque Steam), lancement depuis le dossier du jeu par `sh -c` avec les valeurs en arguments séparés, WebRCON interne |
| `enshrouded` | 1 | serveur Windows sous Proton GE, journal redirigé vers la console (brouillon : `steamcmd` doit pouvoir forcer la plateforme Windows) |
| `arma-3` | 1 | `steamcmd` avec le compte Steam de l'installation (`login: account`), `server.cfg` en `template` regénéré à chaque démarrage, `basic.cfg` libre (brouillon : pas encore essayé sur un vrai nœud) |
| `fivem` | 1 | build épinglé en `download` + `extract` d'un `.tar.xz` (liens internes recopiés), port `tcp+udp` sur une allocation, `kuro.cfg` en `template` chargé par un `server.cfg` libre, ressources de base à un commit fixé (brouillon : pas encore essayé avec une vraie licence) |
| `redm` | 1 | variante RedM (Red Dead Redemption 2) du prefab `fivem`, même installation, jeu `rdr3` (brouillon : pas encore essayé avec une vraie licence) |
| `counter-strike-2` | 1 | `steamcmd` (app 730, anonyme) puis `script` (bibliothèque Steam), lancement `sh -c` avec arguments séparés, mode de jeu par `+game_alias`, jeton GSLT facultatif (brouillon : pas encore essayé) |
| `nextcloud` | 2 (`web`, `db`) | site web, installation par le point d'entrée de l'image, `template` PHP avec `{{ server.web_host }}`, actions `occ` dont `cron` planifiable, arrêt par le signal de l'image (brouillon) |
| `owncloud-ocis` | 1 | site web, `ocis init` en `script` dans l'image officielle (point d'entrée vidé), configuration générée gardée à la réinstallation (brouillon) |
| `openclaw` | 1 | site web, `openclaw.json` en `managed-keys` JSON (origine du site, proxy de confiance), actions d'approbation d'appareil, prêt par healthcheck (démarre sur un nœud Linux ; interface publique pas encore essayée) |
| `hermes-agent` | 1 | site web, image s6-overlay lancée sans root (`/run` en `writable`), `config.yaml` en `managed-keys` YAML, arrêt par le signal de l'image (démarre sur un nœud Linux ; HTTPS public pas encore essayé) |

## Structure

```
<dossier>/
├── prefab.yaml         manifest (format kuro.prefab/v1)
├── locales/en.json     obligatoire (langue de repli), puis fr.json…
├── templates/          gabarits de fichiers
└── assets/             icône, bannière, vignette (PNG, JPEG ou WebP)
```

Le nom du dossier est libre ; l'identifiant du prefab est l'`id` de son manifest. Un prefab publié
est **immuable** par `id` + `version` : changer son contenu demande une nouvelle version.

Pour l'éditer avec autocomplétion et validation (VS Code, extension YAML de Red Hat), en tête de
`prefab.yaml` :

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/kuroctl/kuro/main/shared/prefab/schema/prefab.v1.schema.json
```

## Vérifier

Le JSON Schema ne couvre pas tout (références de gabarits, chemins sous les volumes, bornes des
valeurs par défaut, clés de traduction). La CI charge chaque dossier exactement comme l'import de
Kuro, avec la crate `kuro-prefab` ; en local, depuis un clone de Kuro :

```bash
cargo run -p kuro-prefab --bin kuro-prefab-check -- ../kuro-prefabs
```

## Images

`assets/` : visuels des éditeurs (Steam pour les jeux Steam) ou de la communauté, à remplacer si
leur usage n'est pas autorisé pour votre déploiement.

Un jeu vendu sur Steam déclare `steam: { app: <id> }` : l'identifiant **du jeu** sur le magasin
(730 pour Counter-Strike 2, 892970 pour Valheim), pas celui de son serveur dédié. Le Studio en
importe alors les images (« Importer depuis Steam »). Comme `assets`, cette rubrique est
cosmétique : l'ajouter ne demande pas de nouvelle version.
