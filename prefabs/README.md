# Prefabs officiels

Modèles de serveurs livrés avec Kuro. Format : [ADR-0011](../docs/decisions/ADR-0011-prefabs.md).

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
| `arma-3` | 1 | `steamcmd` avec le compte Steam de l'installation (`login: account`, ADR-0033), `server.cfg` en `template` regénéré à chaque démarrage, `basic.cfg` libre (brouillon : pas encore essayé sur un vrai nœud) |
| `fivem` | 1 | build épinglé en `download` + `extract` d'un `.tar.xz` (liens internes recopiés), port `tcp+udp` sur une allocation, `kuro.cfg` en `template` chargé par un `server.cfg` libre, ressources de base à un commit fixé (brouillon : pas encore essayé avec une vraie licence) |

Chaque prefab est un dossier :

```
<id>/
├── prefab.yaml         manifest, validé par schema/prefab.v1.schema.json
├── locales/en.json     obligatoire (langue de repli), puis fr.json…
├── templates/          gabarits de fichiers
├── assets/             icône, couverture
└── README.md
```

Pour l'éditer avec autocomplétion et validation (VS Code, extension YAML de Red Hat), en tête de `prefab.yaml` :

```yaml
# yaml-language-server: $schema=../schema/prefab.v1.schema.json
```

Les contrôles qu'un JSON Schema ne peut pas exprimer (références de gabarits, chemins sous les volumes, bornes des valeurs par défaut, clés de traduction) seront faits par la crate `shared/prefab`, à l'import comme dans le Studio.

Images (`assets/`) : visuels des éditeurs (Steam pour les jeux Steam), à remplacer si leur usage n'est pas autorisé pour votre déploiement.
