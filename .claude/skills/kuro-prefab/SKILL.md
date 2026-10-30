---
name: kuro-prefab
description: Écrire un nouveau prefab Kuro (modèle de serveur de jeu ou d'application) ou en faire évoluer un, par le MCP `kuro` (Prefab Studio) ou directement dans `prefabs/`. À charger dès qu'on demande de créer, convertir (docker-compose, egg Pterodactyl), corriger ou versionner un prefab, ou d'utiliser les outils `mcp__kuro__*_prefab_*`.
---

# Écrire un prefab Kuro

Un prefab est un dossier : `prefab.yaml` (manifest `kuro.prefab/v1`), `locales/en.json` (obligatoire,
langue de repli) et `locales/fr.json`, `templates/` (gabarits), `assets/` (images). Le format fait
foi dans `shared/prefab/schema/prefab.v1.schema.json` ; la décision dans
`docs/private/decisions/ADR-0011-prefabs.md`. Ce skill dit **comment aller vite sans se tromper**,
il ne recopie pas le schéma : en cas de doute, le schéma et la validation gagnent.

## Démarche

1. **Partir d'un prefab proche, jamais d'une page blanche.** Choisir le modèle dans le tableau
   ci-dessous et lire son `prefab.yaml` et ses locales en entier (les commentaires expliquent les
   choix).
2. **Changer l'identité tout de suite** : `id` (slug unique, nom du jeu), `version: 1.0.0`, `tags`,
   `prefab.name` et `prefab.description` dans chaque locale. Deux prefabs avec le même
   `id@version` entrent en conflit au rechargement du catalogue (cas déjà vécu : la copie RedM de
   `fivem` qui gardait `id: fivem`).
3. **Écrire le manifest** section par section (checklist plus bas), en sourçant chaque valeur
   propre au jeu (ports, binaire, ligne « serveur prêt », commande d'arrêt) depuis la doc officielle
   du serveur dédié, un egg Pterodactyl ou l'image Docker utilisée. Ne pas inventer une ligne de
   console : la noter comme à vérifier.
4. **Valider en boucle** jusqu'à zéro problème, en corrigeant d'après `issues[].path`.
5. **Marquer `x-status: "draft: <ce qui reste à vérifier>"`** tant que le prefab n'a pas tourné sur
   un vrai nœud : il n'est alors déployable que sur un nœud de test.
6. **Rendre la main** : la publication, les images et l'essai sur nœud de test restent des gestes
   humains dans le panel. Résumer ce qui a été sourcé et ce qui reste à vérifier en jeu.

## Par le MCP `kuro` (Prefab Studio)

| Étape | Outil |
|---|---|
| Voir le catalogue | `list_prefabs` |
| Lire le format | `get_prefab_schema` (une fois par session suffit) |
| Lire un modèle | `create_prefab_draft` avec `from: {id, version}` : rend ses fichiers, commentaires compris |
| Convertir un compose | `create_prefab_draft` avec `compose` (+ `id`) ; lire les notes : adapté, ignoré, refusé |
| Valider sans écrire | `validate_prefab_files` avec tous les fichiers |
| Enregistrer | `save_prefab_draft` (remplace **tous** les fichiers du brouillon) |
| Relire | `get_prefab_draft` |
| Images d'un jeu Steam | `import_prefab_draft_steam_images` (lit `steam.app` du manifest **enregistré** ; SteamGridDB d'abord si sa clé est réglée, sinon le magasin Steam) |

Pièges :
- `from` crée la **version suivante du même prefab** (`1.0.0` → `1.0.1`). Pour un nouveau jeu,
  réécrire `id` et remettre `version: 1.0.0` avant d'enregistrer, sinon c'est une nouvelle version
  du modèle.
- `from` reprend aussi les **images** du modèle : sur un nouveau prefab, prévenir l'utilisateur
  qu'elles sont à remplacer dans le panel (le MCP ne dépose pas d'image).
- Plutôt que d'ouvrir un modèle pour le lire puis un brouillon vierge, réutiliser le brouillon
  copié comme brouillon du nouveau prefab (nouvel `id`) : pas de brouillon orphelin.
- Un brouillon modèle ouvert seulement pour lecture reste dans le Studio : le signaler à
  l'utilisateur (le MCP ne sait pas encore supprimer un brouillon).
- `save_prefab_draft` et `validate_prefab_files` attendent **tous** les fichiers texte
  (`prefab.yaml`, `locales/en.json`, `locales/fr.json`, `templates/…`) ; un fichier omis est
  supprimé du brouillon.
- Noter chaque `draft_id` créé : il n'y a pas d'outil pour les lister.
- Pas d'import d'egg, d'essai ni de lecture de console par le MCP pour l'instant : partir de
  l'egg à la main, et laisser l'essai à l'humain.
- Si le MCP ne répond pas (`ECONNREFUSED`) : l'API et le panel de dev ne tournent pas (`cargo xtask
  dev`, puis MCP ouvert dans Paramètres › Projet). Le dire plutôt que contourner.

## Dans le dépôt (prefabs officiels)

`prefabs/` est le sous-module `kuroctl/prefabs` (ADR-0041) : un prefab officiel s'écrit dans ce
dépôt, pas dans Kuro. Écrire le dossier `prefabs/<dossier>/` (en-tête `# yaml-language-server:
$schema=https://raw.githubusercontent.com/kuroctl/kuro/main/shared/prefab/schema/prefab.v1.schema.json`,
puis un commentaire qui explique le choix d'image et d'installation). Ensuite :
- ajouter sa ligne dans `prefabs/README.md` ;
- vérifier : `cargo run -p kuro-prefab --bin kuro-prefab-check -- prefabs` (chargement comme
  l'import, images, `id@version` en double), puis `cargo test -p kuro-prefab` et
  `cargo test -p kuro-api --test features servers::prefab` (les listes de prefabs officiels sont
  lues sur disque, rien à mettre à jour) ;
- commit et push dans `kuroctl/prefabs`, puis Kuro avance le pointeur du sous-module ; le
  commit est fait par l'utilisateur.

Un prefab officiel publié est **immuable** : changer son contenu demande une nouvelle version.

## Quel modèle copier

| Cas | Modèle | Ce qu'il montre |
|---|---|---|
| Binaire téléchargé (zip/tar), image Debian nue | `terraria` | `download` + `extract` épinglés sha256, `managed-keys` ini, sauvegarde à chaud |
| Archive `.tar.xz`, port TCP+UDP | `fivem-gta5` | `extract` xz, `tcp+udp`, `template` regénéré + fichier libre |
| Jeu Steam Linux, image Pterodactyl (`STARTUP`) | `valheim` | `steamcmd`, réglages par l'environnement, arrêt SIGINT |
| Jeu Steam + bibliothèque à copier | `rust` | `steamcmd` puis `script`, lancement `sh -c` avec arguments séparés |
| Jeu Steam + mods Workshop | `project-zomboid` | `managed-keys` sans sections, `reference` Workshop, `fields` / `prefill` |
| Jeu Steam exigeant un compte | `arma-3` | `login: account` (ADR-0033), `template` regénéré |
| Serveur Windows uniquement | `enshrouded` | Proton GE, `platform: windows`, journal vers la console |
| Image qui s'installe seule | `minecraft-java` | `managed-keys` properties, mods Modrinth/CurseForge, `backup` complet |
| UDP seul (RakNet), add-ons | `minecraft-bedrock` | un port UDP, `extract` de packs |
| Mods CurseForge, jeu Steam | `ark-survival-ascended` | `reference` CurseForge, fichiers ini mixtes |
| Application web multi-services | `wordpress` | réseau privé entre services, secret généré partagé, healthcheck |

Pour l'exposition web (Caddy, port 80, domaine joignable) : lire la section correspondante du
schéma, le format est en cours d'évolution ; ne pas l'inventer.

## Checklist du manifest

**Identité** : `schema: kuro.prefab/v1`, `id`, `version`, `category` (`game`, `web`, `database`,
`tool`, `other`), `tags`, `authors`, `requires.kuro`, `requires.resources.memory_mb` /
`disk_mb` (`min` et `recommended`, ADR-0031 : un serveur sous le minimum est refusé). Jeu vendu sur Steam : `steam: { app: <id> }`, l'identifiant **du jeu** sur le magasin (Valheim 892970, pas son serveur 896660), vérifiable par `IStoreBrowseService/GetItems` ; il permet d'importer ses images.

**Services** :
- `image` épinglée : tag daté ou `@sha256:` (jamais `latest`, ni un tag qui bouge comme
  `yolks:debian`). Le runtime de l'image suit le jeu (Java 25 pour Minecraft récent).
- `user: "1000:1000"` ; racine en lecture seule : tout ce qui écrit hors du volume va dans
  `writable` (`/tmp`…). Un jeu qui écrit dans `~` plante : `env: { HOME: /data }`.
- `ports` nommés (`game`, `query`, `rcon`…), `protocol` `tcp` / `udp` / `tcp+udp`, `public: true`
  seulement pour ce que les joueurs joignent. Un RCON reste interne.
- `volumes` : un volume de données (`/data`) ; les chemins de fichiers et d'installation sont sous
  un volume.

**Installation** : chaque `download` a son `sha256` ; `extract` rend les binaires exécutables
(`executable: [...]`) ; nettoyer l'archive (`remove`). Préférer les étapes typées à `script`.

**Settings** : sections `general` / `advanced`, groupes avec `icon`, champs typés. Mot de passe saisi
par l'utilisateur en `password` ; secret généré au déploiement (RCON, admin) en
`secret` avec `generate: { length: 24 }`. `restart: true` si la valeur n'est lue qu'au démarrage,
`editable: deploy` si elle n'a de sens qu'à la création (taille du monde). Bornes (`min`, `max`,
`max_length`, `pattern`) sur tout ce qui finit dans un fichier ou une ligne de commande.

**Fichiers** : `managed-keys` quand le jeu a son propre fichier que l'utilisateur peut aussi
éditer ; `template` quand Kuro le regénère entièrement à chaque démarrage ; `user` pour un fichier
libre. Valeurs via `{{ settings.x }}`, `{{ ports.game.container }}`. Rien de concaténé dans une
ligne de commande quand un fichier de config peut porter la valeur (le point d'entrée Pterodactyl
découpe `STARTUP` sur les espaces).

**Cycle de vie** : `ready` = regex sur une ligne réellement affichée quand le serveur accepte les
joueurs ; `stop` = commande console propre (sinon signal) ; `timeout` réaliste.

**Sauvegarde** (`backup.before`) : le `wait` vise la ligne affichée **à la fin** de
l'enregistrement, sinon chaque sauvegarde attend 60 s. À vérifier en jeu.

**Locales** : `en.json` et `fr.json` complets et symétriques : `prefab.name` / `description`, un
titre par groupe, un `label` (et `help` utile) par champ, les `options` de chaque `enum`, une
description par fichier. Une clé manquante est une erreur de validation.

**Images** : `icon`, `cover`, `thumbnail` en PNG, JPEG ou WebP (jamais SVG, qui fait refuser le
prefab entier), chemins dans `assets:` égaux aux vrais noms de fichiers.

## Pièges connus

Lire la section « Prefabs » de `docs/private/developpement.md` avant un premier prefab ; y ajouter
tout nouveau piège rencontré.

Un manque du format, du MCP ou du daemon (contournement nécessaire, outil absent) se note dans
`docs/private/internals/retours-prefabs.md` avec le prefab qui l'a montré : ce relevé est transmis
à qui fait évoluer Kuro. Ne pas modifier le format ou le daemon soi-même pendant l'écriture d'un
prefab.
