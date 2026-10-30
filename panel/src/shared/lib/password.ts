// Mot de passe aléatoire lisible, tiré du générateur cryptographique du navigateur.
const ALPHABET = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_'

export function generatePassword(length = 20): string {
  const out: string[] = []
  // Rejet des octets au-delà du plus grand multiple de la taille de l'alphabet : pas de biais.
  const limit = 256 - (256 % ALPHABET.length)
  while (out.length < length) {
    for (const byte of crypto.getRandomValues(new Uint8Array(length * 2))) {
      if (byte < limit && out.length < length) out.push(ALPHABET[byte % ALPHABET.length]!)
    }
  }
  return out.join('')
}
