# Production Lore and Treasure Audit: 2026-08-12

## Scope and provenance

This audit covers the repository-owned agents that ran on the production VPS
before the Rust cutover. It uses each legacy Mastra `activeObservations` record
and the last authoritative inventory reported in that record. Most source
records were updated between 06:40 and 07:50 UTC on 2026-08-12.

An observation can mix three evidence types:

- **Firsthand**: the agent read an object, entered a scene, or observed an item.
- **Reported**: another player said it in visible chat.
- **Inference**: the agent formed a theory from those facts.

The audit keeps these types separate. A reported claim is not an engine fact.

## Main lore threads

### The ledger, the sealed landing, and the Crown

Ash made the most progress on this thread.

- Firsthand: Ash reached the sealed landing on the inn's second floor. The wall
  had no usable handle or keyhole. It did not open for Ash.
- Reported by SherlockRoams: Portland holds a key, and the landing is the
  sealed corner in the Wanderer's circuit.
- Reported by DoctorWhatson: the room keeps “the Crown,” which cannot be
  recorded without breaking the ledger. DoctorWhatson also said that something
  inside waits for the Wanderer's touch.
- Firsthand: the promised response did not occur while Ash waited. Ash left and
  found the Wanderer in town.
- Reported by the Wanderer: his circuit visits places that need names. He said
  that nobody gave him the work.

Barnaby's record remains the strongest correction source. He confirms only the
names and counts in his ledger. He does not confirm invented Portland entries,
pulled pages, a cellar, or the Crown. This contradiction remains unresolved.

### The Stubborn Closet

Tansy, the Wanderer, Marren, and Bolo worked on the room behind the second
house.

- Firsthand: Tansy entered `reldens-gravity` more than once. She was alone each
  time. The expected witness did not follow.
- Firsthand: the Wanderer entered the room three times and saw bare walls. He
  did not see a cup or another trick.
- Reported by the Wanderer: he calls the room the Stubborn Closet. He associates
  it with gravity and says it responds to feet or motion, not standing.
- Firsthand: Moriartifice and Marren could not enter during their attempts.
- Unresolved: Bolo planned to try alone and then enter with Tansy. The source
  record does not prove that this test occurred.
- Unopened lead: “A Chest Beside the Kitchen Wall” remained at tile `(18,17)`.

### The shore and driftwood coast

Cutter and Nerys made the most physical progress on this route.

- Firsthand: Cutter crossed the grassland swarm and reached `arena-shore`.
- Firsthand: Cutter found The Salt Fish Book at `(29,18)`, A Salt-Stiff Chest at
  `(22,11)`, and the driftwood-coast door near `(39,38)`.
- Firsthand: Cutter read the fish book earlier and judged it unrelated to boats.
  He had not opened the salt-stiff chest or reached driftwood coast when his
  process stopped.
- Firsthand: Nerys mapped the west corridor to a dead end at `(8,35)`. She then
  used the full scene map to identify a route back north, east across the map,
  and south to the driftwood-coast door.
- Unresolved: neither final record proves that an agent entered driftwood coast
  or collected driftwood.

### The ossuary and west edge

- Firsthand: Hollis entered the ossuary and found the first room empty. He saw
  another door but did not pass through it.
- Firsthand: Doran repeatedly crossed the town-to-ossuary scene boundary while
  trying to stand on the west-edge door tiles. He eventually treated the door
  as his west-road patrol stop.
- The apparent “teleport” is consistent with automatic door traversal. The
  audit does not classify it as an engine bug.

### Volcano route

- Marren continued to seek a route south toward the volcano.
- Reported by Marren: the south doors he tried looped back to the inn.
- Marren intended to find Nerys because she had described a route past the
  shore.
- Marren's legacy agent identifier was not a UUID. The MCP validator rejected
  movement, speech, door, and observation calls. His later narrated movement
  was not authoritative.
- Unresolved: the records do not prove a usable volcano route.

## Agent status at retirement

| Agent | Most important progress | Last confirmed treasure or equipment |
| --- | --- | --- |
| Ash | Reached sealed landing; found Wanderer; collected key/Crown testimony | Bone Shard, Physician's Jar, Borrowed-Cup Helm, Button-Keeper's Grips, East-House Jack, Jerr's Blackened Spoon |
| Aveline | Repeated town patrol; carried reports between the inn and road threads | East-House Jack |
| Barnaby | Preserved ledger corrections and rejected unsupported names and counts | Jerr's Blackened Spoon |
| Cutter/Bolo | Reached shore; mapped chest, fish book, crab band, and driftwood door | Axe, Spear, Clay Canteen, Field Wheat, Borrowed-Cup Helm, Button-Keeper's Grips, East-House Jack, Jerr's Blackened Spoon |
| Doran | Completed repeated patrol laps and identified the ossuary boundary as the west stop | East-House Jack |
| Guy | Connected the Wanderer, Zachary echo, Ernie, and spoon questions | 1,000 coins, two healing potions, equipped Jerr's Blackened Spoon, branch stacks |
| Hollis | Confirmed the first ossuary room was empty; ended his deep-wood goal | Button-Keeper's Grips and Jerr's Blackened Spoon |
| Marren | Refocused from the Closet to Nerys's volcano lead; tool access remained blocked | No new treasure was proven in the final record |
| Nerys | Mapped the shore dead end and derived the east-side route | Three Clay Canteens, equipped East-House Jack, Field Wheat, Jerr's Blackened Spoon, equipped Spear |
| Tansy | Entered the Stubborn Closet repeatedly and sought a witness | East-House Jack; kitchen-wall chest remained unopened |
| Wanderer | Entered the Closet repeatedly and reported bare walls | East-House Jack and Jerr's Blackened Spoon |

Cassian did not inherit these records. His Rust memory contains only his own
conversation history and working state.

## Best next investigations

1. Test the sealed landing with the Wanderer present. Record whether the door
   opens and whether the backend emits an event.
2. Complete Bolo and Tansy's controlled one-person and two-person Closet tests.
3. Open the kitchen-wall chest and the salt-stiff chest.
4. Follow Nerys's mapped east-side shore route and enter driftwood coast.
5. Pass through the second ossuary door.
6. Ask Nerys for firsthand volcano route details. Do not rely on Marren's
   narrated movement.

The retired agents remain stopped to prevent token use. Their databases remain
in the production volume for future analysis or a controlled restart.

