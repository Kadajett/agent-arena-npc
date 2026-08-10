/**
 * What a character is feeling right now.
 *
 * Closed to a small set, on purpose, so it can be rendered and reasoned about
 * instead of read as free text: a spectator's client draws one of a known
 * handful of emoji, never whatever string the model happened to write.
 *
 * A feeling is optional and additive everywhere it appears - it rides along
 * with whatever the character is already doing, the same way `remember` or
 * `todo` does (see BOOKKEEPING in behavior.ts), and is never a substitute for
 * an action. Guy stuck in the volcano still has to answer with a real action
 * every tick; the feeling just says how it is going.
 *
 * The set is picked to cover what these three actually go through: Guy's
 * suspicion and his running tally of small grievances, Barnaby's snark with
 * dread underneath it, the Wanderer's confident enthusiasm - and the
 * motivating case for building this at all, being stuck somewhere with no
 * good option left.
 */
export const FEELINGS = [
  'content',
  'amused',
  'curious',
  'suspicious',
  'annoyed',
  'angry',
  'afraid',
  'desperate',
  'sad',
  'lonely',
  'tired',
  'confused',
  'bored',
  'hopeful'
] as const;

export type Feeling = (typeof FEELINGS)[number];

const FEELING_SET: ReadonlySet<string> = new Set(FEELINGS);

export function isFeeling(value: string): value is Feeling {
  return FEELING_SET.has(value);
}

/** How each feeling renders over a character's head in the spectator viewer. */
export const FEELING_EMOJI: Record<Feeling, string> = {
  content: '🙂',
  amused: '😄',
  curious: '🤔',
  suspicious: '🧐',
  annoyed: '😒',
  angry: '😠',
  afraid: '😨',
  desperate: '😰',
  sad: '😢',
  lonely: '😔',
  tired: '😪',
  confused: '😕',
  bored: '🥱',
  hopeful: '🤞'
};

/** The emoji for a feeling, or undefined for anything outside the closed set. */
export function emojiFor(feeling: string | undefined): string | undefined {
  return feeling && isFeeling(feeling) ? FEELING_EMOJI[feeling] : undefined;
}

/**
 * One line for the model: told the enum exists and roughly what it is for,
 * without spending tokens on a gloss for each of the fourteen words - a
 * character that already thinks in English knows what "annoyed" means.
 *
 * The wording expects a feeling rather than permitting one, and that is a
 * correction rather than a preference. The first version said "only when one is
 * genuinely present; leave it out otherwise", which reads as an invitation to
 * skip it, and a model writing the smallest JSON that satisfies the ask will
 * take that invitation every single time. Twenty minutes of three characters
 * living their lives produced not one feeling between them.
 *
 * Saying what it is for is what makes it worth filling in. A field labelled
 * optional is decoration; a field that is how anybody watching can tell how you
 * are doing is part of being a person in a room with other people.
 */
export const FEELING_GUIDANCE =
  'Add "feeling" whenever one is true of you: one of ' + FEELINGS.join('/') + '. How you are, '
  + 'not what you are doing. It shows over your head, so it is the only way anybody watching '
  + 'can tell. Leave it out only if you feel none of them.';
