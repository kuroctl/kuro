/** Teinte stable (0–359) tirée de l'id d'un prefab : même couleur partout pour un même jeu. */
export function prefabHue(id: string): number {
  let hash = 0
  for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) >>> 0
  return hash % 360
}
