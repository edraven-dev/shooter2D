const ADJECTIVES = [
  'Brave',
  'Swift',
  'Quiet',
  'Lucky',
  'Crafty',
  'Bold',
  'Sly',
  'Grim',
  'Rapid',
  'Steady',
  'Wild',
  'Keen',
];
const ANIMALS = [
  'Otter',
  'Falcon',
  'Badger',
  'Ferret',
  'Lynx',
  'Raven',
  'Viper',
  'Wolf',
  'Mantis',
  'Gecko',
  'Bison',
  'Heron',
];
const STORAGE_KEY = 'shooter2d.name';

/** A name like "SwiftOtter". */
export function randomName(random: () => number = Math.random): string {
  const pick = (words: readonly string[]): string => words[Math.floor(random() * words.length)] ?? words[0]!;
  return pick(ADJECTIVES) + pick(ANIMALS);
}

/** The name the player used last time, or a fresh random one. */
export function loadName(): string {
  try {
    const saved = localStorage.getItem(STORAGE_KEY)?.trim();
    if (saved) {
      return saved;
    }
  } catch {
    // Storage can be unavailable (private windows, blocked site data).
  }
  return randomName();
}

export function saveName(name: string): void {
  try {
    localStorage.setItem(STORAGE_KEY, name);
  } catch {
    // Not remembering the name is fine.
  }
}
