/**
 * What each kind of character can actually do, beyond swinging at things.
 *
 * This has to live here because nothing else knows it. The gateway has no
 * concept of a skill at all: arena_use_action takes whatever action_type it is
 * handed and passes it straight through, and the world is the only thing that
 * checks whether the character asking is allowed. So a character with no list
 * of its own would either never cast anything, which is what was happening, or
 * guess and be refused somewhere it cannot see the refusal.
 *
 * The lists below are the skills_class_path_level_skills rows as they stand,
 * read back out of the live database rather than assumed:
 *
 *   journeyman  attackBullet, attackShort, fireball, heal
 *   sorcerer    attackBullet, fireball, heal
 *   swordsman   attackShort, heal
 *   warlock     attackBullet, attackShort, fireball
 *   warrior     attackBullet, attackShort, heal
 *
 * If somebody adds a class path or moves a skill between them, this goes stale
 * silently, which is the one bad property of keeping it here. The check is one
 * query and it is written out in skills.test.mjs so it can be re-run.
 *
 * Worth knowing while reading this: no class path changes how a character
 * looks. Every spritesheet in this world has four animations and all four are
 * walking. A cast is drawn from effects keyed by the skill, not by the costume,
 * which is why giving Nerys robes did nothing and giving her fireball does.
 */
export const SKILLS_BY_CLASS_PATH: Record<string, readonly string[]> = {
  journeyman: ['attackBullet', 'attackShort', 'fireball', 'heal'],
  sorcerer: ['attackBullet', 'fireball', 'heal'],
  swordsman: ['attackShort', 'heal'],
  warlock: ['attackBullet', 'attackShort', 'fireball'],
  warrior: ['attackBullet', 'attackShort', 'heal']
};

/**
 * Journeyman is the fallback because it is also the fallback used when a
 * character registers without naming a class path, in npc.ts. The two have to
 * agree or a character would be told it knows things the world will refuse.
 */
export const DEFAULT_CLASS_PATH = 'journeyman';

export function skillsFor(classPath: string | undefined): readonly string[] {
  return SKILLS_BY_CLASS_PATH[classPath ?? DEFAULT_CLASS_PATH] ?? [];
}
