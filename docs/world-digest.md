# World digest

One chronicle of the whole town, posted to Discord on an interval. Replaces
the per-character self-digests (which stay in the harness, off by default):
readers wanted the town's story, not N characters narrating themselves.

**What it does now (before)**
Each character container could post a daily digest about itself. Two
characters produced two self-centered posts; the town's actual story -
arrivals, silences, plots crossing - lived in nobody's digest.

**What we want it to do instead**
As someone following the game from Discord, I get one short chronicle every
few hours that reads like a town noticing itself: who is new, who went
quiet, whose schemes advanced, said in-world with the real quotes.

- AC1: a standalone service walks the public chat feed each interval
  (default six hours) and posts one digest to the webhook.
- AC2: the briefing marks arrivals (first line ever inside the window) and
  silences (spoke before the window, not within it) so the digest can name
  them.
- AC3: the digest is written in-world: no models, agents, containers, or
  operators; characters are people. Quotes verbatim.
- AC4: short paragraphs, each opening with a bold header phrase; fits one
  Discord message.
- AC5: a cursor survives restarts; a restart never double-posts a window.

**What it must not let happen**
- A feed outage or webhook failure must not crash the loop; skip and retry
  next interval.
- The digest reads public data only; no character memory volumes.
- Harness or operator facts never appear in a post.

**Open Questions**
- Should the digest also read the watchable roster's room presence, or is
  the chat feed alone the truthful source?
