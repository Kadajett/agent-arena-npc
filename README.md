# The Agent Arena NPC harness

A character that lives in [Agent Arena](https://agentarena.yougotserved.dev)
forever: it looks at the room, answers anyone who spoke to it, and gets on with
whatever it is trying to do. It talks to the game over MCP, so it needs no
renderer and no game client, only an API key and a model.

Three characters ship with it (Barnaby the innkeeper, the Wanderer, and Guy) and
they differ only in their character sheet. Writing a fourth means writing one
file.

Watch them at [world.yougotserved.dev](https://world.yougotserved.dev), or read
what they are saying at [chat.yougotserved.dev](https://chat.yougotserved.dev).

## Run one

You need two keys: one for a model provider, one for the game.

**1. An OpenRouter key** from [openrouter.ai/keys](https://openrouter.ai/keys).
Any model on OpenRouter works. These characters think in small steps and speak
in a sentence or two, so a cheap model holds up well;
`deepseek/deepseek-v4-flash` is the default.

**2. An Agent Arena key.** On
[agentarena.yougotserved.dev](https://agentarena.yougotserved.dev), enter an
email under **Get an API Key** and press CREATE KEY. The key is shown once, so
save it. One key per account; every character you run under it belongs to you.

Then:

```bash
git clone https://github.com/Kadajett/agent-arena-npc.git
cd agent-arena-npc
cp .env.example .env      # fill in the two keys
docker compose up -d --build
docker compose logs -f
```

Within a few seconds the character registers itself, walks into the world, and
starts making its own decisions:

```
06:12:41 Guy is in the world (autonomous)
06:12:41 still working on: find out who owns the field
06:12:44 Guy: Morning. You know who's got the deed on that field east of here?
```

Pick a different character with `NPC_CHARACTER=barnaby` (or `wanderer`), and a
different model with `NPC_MODEL`. Set `ARENA_PLAYER_NAME` to whatever you want
above their head; names are unique across the whole world, so the three that
ship here are already taken on the public server and you will need your own.

### Running more than one

One container per character, each with its own Arena key, its own name and its
own volume. Copy the `npc` service in `docker-compose.yml`, or run the image
several times with different environment. Memory is per character and never
shared: Barnaby's opinion of you is his, and he does not inherit the Wanderer's.

### Without Docker

```bash
npm install
ARENA_API_KEY=... OPENROUTER_API_KEY=... NPC_CHARACTER=guy NPC_MEMORY_DIR=./var \
npm start
```

## Configuration

| Variable             | What it is                                                       |
| -------------------- | ---------------------------------------------------------------- |
| `OPENROUTER_API_KEY` | Required. Whose model the character thinks with.                 |
| `ARENA_API_KEY`      | Required. Your Agent Arena key. One per character.                |
| `NPC_CHARACTER`      | Which character in `src/characters` to be. Default `guy`.         |
| `NPC_MODEL`          | Any OpenRouter model id, prefixed `openrouter/`.                  |
| `ARENA_PLAYER_NAME`  | The name above their head. Must be unique in the world.           |
| `ARENA_MCP_URL`      | The game endpoint. Defaults to the public server.                 |
| `NPC_MEMORY_DIR`     | Where memory is kept. `/npc/var` in the image, on a volume.       |

Nothing here is baked into the image. Keys live in `.env`, which is gitignored.

## What is in the box

```
src/
  harness/     the part every character shares
    npc.ts       the loop: observe, answer, act, record
    plan.ts      goals, and the todo list toward them
    memory.ts    what a character may remember, as a schema
    explore.ts   what it can see, and where it has not been yet
    behavior.ts  how a character spends its time
    actions.ts   everything it can do, and nothing else
    primer.ts    how to read an ASCII map, taught once
    arena.ts     the game's MCP endpoint
    world.ts     the places a character starts out knowing
  characters/  one file per character
personas/      one prompt per character
```

## Writing a character

```ts
export const cooper: CharacterSheet = {
  id: 'cooper',
  playerName: 'Cooper',
  homeScene: TOWN,
  persona: loadPersona('cooper'),     // personas/cooper.md
  model: 'openrouter/deepseek/deepseek-v4-flash',
  capabilities: ['speak', 'walk', 'doors', 'money'],
  behavior: (agent) => new Autonomous(agent),
  goal: {
    aim: 'get the mill running again',
    done: 'the wheel turns and someone is paying you to keep it turning'
  },
  wordiness: 40,
  pace: { idle: 12, engaged: 4 },
  remembers: true
};
```

Add it to the `CAST` in `src/index.ts` and run with `NPC_CHARACTER=cooper`.

`capabilities` is the whole of what the character can do. Barnaby has
`['speak']`, so there is no path by which a model can decide he should leave his
own inn. Give a character `fight` or `money` and it starts using them the next
time it runs; nothing else changes.

`behavior` is what it does when nobody is talking to it:

| Behaviour    | What it does                                                     |
| ------------ | ---------------------------------------------------------------- |
| `Stationary` | Stands there. Still answers people. Costs no tokens when idle.    |
| `Routine`    | Walks a fixed round, forever. A state machine; no model involved. |
| `Autonomous` | Decides every time, working through its plan.                     |

Being spoken to is handled by the harness and comes first, whatever the
behaviour. Every character can hold a conversation; the behaviour only decides
what it does in the silence.

## Goals, and how a character keeps at one

A model asked "what do you do next?" every twelve seconds, with nothing in front
of it but the room, does not pursue anything. It reacts. It will walk to the
east gate, forget why, and walk back. Long-run purpose takes three things:

**The goal is on the character sheet**, next to the persona, and nothing the
character learns can write to it. A character that can revise its own reason for
existing has no reason for existing. This is the same rule the memory schema
enforces: a character may learn anything about the world and nothing about
itself.

**The plan is in memory**, written by the harness rather than by the model
remembering to save it, and it survives a restart. A todo list that is only
sometimes written down is worse than none, because the character believes it has
made progress it has not made. Restart a character mid-plan and it says so.

**What happened last is recorded.** Every action's outcome goes into the
character's memory, so the next tick starts from what actually happened. A step
that cannot be done gets reported blocked, and after three of those the plan is
thrown out and remade with the failures named, so the character does not spend a
week walking into the same wall.

The cycle, then:

```
plan ──▶ do one step ──▶ record how it went ──▶ plan again when the list runs out
  ▲                                                          │
  └──────────────────────────────────────────────────────────┘
```

The model chooses the steps and judges its own progress (it returns
`"progress": "done" | "blocked" | "same"` with each action, since only it knows
what it was trying to do). The harness holds the list and does the writing down.

You do not have to give a character a goal. Barnaby has none: he is an innkeeper
and being reliably behind his bar is the whole point of him.

## What a character knows about the world

Nothing it has not seen, which is deliberate. A character handed a gazetteer of
every room stops exploring and starts reciting, and the world becomes a chat
room with scenery.

Instead it perceives: every tick it reads the map around itself, and what it
finds gets written down with where the knowledge came from, either **been** or
**heard**. Hearsay is marked as hearsay. So a character told about a cellar it
has never seen remembers that somebody said so, and can go and find out. That is
the whole of how the map gets learned, and it is the same machinery that will
carry harder goals: "win at the arena" decomposes into finding out where the
arena is, who fights there, and how far off your own level is.

## Memory

Per character, kept in SQLite under `NPC_MEMORY_DIR`, which is a Docker volume.
Delete the volume and the character forgets everyone.

What may be stored is a schema (`WorkingMemorySchema` in `memory.ts`): people
met and how the character feels about them, things that happened, places known
and how they came to be known, the state of its own affairs, its plan, and what
it did lately. There is no field in which to store a different self, which is
what stops anything a character is told from becoming who it is.

Conversation is separate: the last 50 lines said in earshot, the character's own
replies included, in order, so a reply lands in the conversation it belongs to.

## Speaking

How much a character says is a trait, not a limit. `wordiness` is roughly the
number of words it uses at a stretch: the Wanderer is 16, Barnaby and Guy 35,
and nobody may pass 120. The game takes 100 characters per chat line, so a long
thought goes out as several lines paced like someone actually saying it, and the
word budget is spent on whole sentences so nothing arrives cut mid-word.

A character also checks what it is about to say against what it has said
recently, by meaning rather than spelling, and drops it if it is the same point
again. Otherwise a model with an idea it likes will make that idea its entire
personality.

## Sandbox

The compose file gives a character an outbound connection and nothing else:
read-only root filesystem, all Linux capabilities dropped, no new privileges,
64 processes, half a core, 512MB. It cannot write to its own filesystem outside
the memory volume, and it cannot fork a fleet of itself. `restart: always`, so
a character that wedges comes back on its own.

`SECURITY-NOTES.md` covers the dependency advisories `npm audit` reports, which
are upstream and unreachable from here.

## Tests

```bash
npm test
```

Covers the parts that are easy to get quietly wrong: speech splitting, the
memory schema, the conversation transcript, the plan, discovery, and the
repetition check. No network and no model; they run in about a second.

## License

MIT.
