---
paths:
  - "panel/src/**"
  - "panel/tests/**"
---

# Langage d'interface du panel

Règles courtes, appliquées par les composants partagés (`@/shared/ui`, `@/shared/layout`). Chercher
d'abord la pièce qui existe ; ces règles disent quand et comment s'en servir (ADR-0064).

- **Couleurs et états.** Uniquement les tokens du design system (`text-success`, `bg-warning/18`…),
  jamais une couleur Tailwind ni une valeur en dur. L'état d'un serveur passe par
  `ServerStateBadge` et `features/servers/state.ts` ; un écran ne choisit pas lui-même la couleur
  d'un état.
- **Textes.** Tout passe par vue-i18n, en français et en anglais (lint `no-raw-text`). Un pluriel
  passe le nombre en troisième argument : `t(clé, { … }, count)`.
- **Mots.** « Supprimer » détruit, « Retirer » détache, « Révoquer » met fin à un accès. « Nouveau
  X » ouvre une création, « Créer » la valide, « Enregistrer » valide un formulaire.
- **Formulaires.** Un formulaire modifiable se termine par `UnsavedBar` : « Enregistrer » attend
  une modification, et quitter la page avec des modifications demande confirmation (la barre
  s'en charge). Un interrupteur seul s'applique tout de suite.
- **Refus.** Une action que la vue sait impossible le dit avant de partir :
  - jamais permise (permission absente) : l'action est cachée ;
  - empêchée par d'autres objets (nœud qui porte des serveurs, région qui a des nœuds, prefab
    utilisé) : la confirmation s'ouvre en état bloqué, avec `BlockedBy` (chaque objet en cause et
    son lien) et seulement « Fermer » ;
  - empêchée pour une autre raison : bouton désactivé et une ligne à côté qui dit pourquoi (un
    bouton désactivé n'affiche pas d'infobulle).
  L'API reste seule juge : son refus s'affiche quand même par `useApiError`.
- **Limites.** Elles viennent de l'API (ressources du prefab, plafonds des Réglages), jamais de
  nombres écrits dans le panel.
- **Suppression.** Confirmation par `AlertDialog` qui nomme l'objet, bouton destructif qui répète
  le verbe. La zone de suppression vient en dernier sur la page.
- **Chargement.** Un squelette (`Skeleton`) de la forme du contenu ; un rafraîchissement garde ce
  qui est affiché. Une action en cours désactive son bouton et les champs qu'elle touche.
- **Notifications.** Un toast seulement pour un résultat qu'on ne voit pas là où on a agi
  (suppression qui ramène à la liste, opération qui finit plus tard), au passé, avec le nom de
  l'objet : « Nœud dev-2 supprimé. ».
- **États vides.** `Empty` pour une page sans rien ; dans une carte, une ligne suffit (« Tout est
  en ordre. » en `text-success`).
- **Listes.** Une ligne qui ouvre une page est un vrai lien (`RouterLink`) ; les listes de données
  passent par `DataTable`, leur état (recherche, filtres, page) dans l'URL.
- **Navigation.** Une nouvelle page s'ajoute au sous-menu (`shared/layout/sections.ts`) : elle
  apparaît alors aussi dans la palette (`Ctrl+K`), avec la même permission.
- **Tests.** Chaque comportement visible a son test dans `panel/tests/features/` (application
  complète, API simulée par MSW), qui cherche par rôle et par texte comme un utilisateur.
