#!/usr/bin/env python3
"""Watch a live harness JSON log and grade autonomous world interaction.

The watcher is deliberately log-only: it never sends MCP commands and cannot
alter the character. It tolerates interleaved/non-JSON lines from a running
process and reports the causal evidence needed to debug a failed run.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable


def json_lines(path: Path, offset: int) -> Iterable[tuple[int, dict[str, Any]]]:
    with path.open("r", encoding="utf-8", errors="replace") as stream:
        stream.seek(offset)
        while True:
            line = stream.readline()
            if not line:
                break
            position = stream.tell()
            try:
                value = json.loads(line)
            except (json.JSONDecodeError, UnicodeDecodeError):
                continue
            if isinstance(value, dict):
                yield position, value


def attributes(event: dict[str, Any]) -> dict[str, Any]:
    raw = event.get("fields", {}).get("attributes", {})
    if isinstance(raw, dict):
        return raw
    if isinstance(raw, str):
        try:
            parsed = json.loads(raw)
            return parsed if isinstance(parsed, dict) else {}
        except json.JSONDecodeError:
            return {}
    return {}


@dataclass
class Run:
    run_id: str = ""
    character: str = ""
    started_at: str = ""
    last_scene: str = ""
    scenes: list[str] = field(default_factory=list)
    events: Counter[str] = field(default_factory=Counter)
    models: Counter[str] = field(default_factory=Counter)
    failures: Counter[str] = field(default_factory=Counter)
    actions: Counter[str] = field(default_factory=Counter)
    model_calls: int = 0
    input_tokens: int = 0
    output_tokens: int = 0
    cached_tokens: int = 0
    reasoning_tokens: int = 0
    cost_usd: float = 0.0
    health_known: bool = False
    health: int | None = None
    max_health: int | None = None
    hostile_seen: bool = False
    combat_seen: bool = False
    damage_dealt: int = 0
    damage_received: int = 0
    deaths: int = 0
    chat_or_social: int = 0
    last_nav_scene: str = ""
    last_nav_tile_known: bool = False
    last_update: float = field(default_factory=time.time)

    def ingest(self, event: dict[str, Any]) -> None:
        fields = event.get("fields", {})
        if fields.get("message") != "analytics_event":
            return
        self.run_id = fields.get("process_run_id", self.run_id)
        self.character = fields.get("character_id", self.character)
        self.last_update = time.time()
        attrs = attributes(event)
        # The logger stores the causal event name in fields.event_name. Accept
        # the alternate top-level form used by older fixtures too.
        event_name = fields.get("event_name", event.get("event_name", ""))
        self.events[event_name] += 1
        model = attrs.get("actual_model") or attrs.get("requested_model")
        if model:
            self.models[str(model)] += 1
        if event_name in {"model.response_received", "model.call_completed"}:
            self.model_calls = max(self.model_calls, int(attrs.get("agent_model_calls_total", self.model_calls)))
            self.input_tokens = max(self.input_tokens, int(attrs.get("agent_input_tokens_total", self.input_tokens)))
            self.output_tokens = max(self.output_tokens, int(attrs.get("agent_output_tokens_total", self.output_tokens)))
            self.cached_tokens = max(self.cached_tokens, int(attrs.get("agent_cached_input_tokens_total", self.cached_tokens)))
            self.reasoning_tokens = max(self.reasoning_tokens, int(attrs.get("agent_reasoning_tokens_total", self.reasoning_tokens)))
            try:
                self.cost_usd = max(self.cost_usd, float(attrs.get("agent_openrouter_cost_usd_total", self.cost_usd)))
            except (TypeError, ValueError):
                pass
        if event_name in {"model.call_failed", "model.output_parse_failed"}:
            self.failures[str(attrs.get("error_class", event_name))] += 1
        if event_name == "body.action_started":
            self.actions[str(attrs.get("action_kind", "unknown"))] += 1
        if event_name in {"mcp.tool_completed", "mcp.tool_started"}:
            tool = attrs.get("tool")
            if tool in {"arena_say", "arena_feel", "arena_talk_to", "arena_choose", "arena_end_talk", "arena_interact"}:
                self.chat_or_social += 1
        if event_name == "strategy.published":
            self.last_nav_scene = str(attrs.get("navigation_scene") or "")
            self.last_nav_tile_known = bool(attrs.get("navigation_tile_known", False))
        if event_name == "perception.frame_published":
            scene = str(attrs.get("scene") or "")
            if scene and scene != self.last_scene:
                self.scenes.append(scene)
                self.last_scene = scene
            self.health_known = bool(attrs.get("health_known", False))
            if self.health_known:
                self.health = attrs.get("health")
                self.max_health = attrs.get("max_health")
            self.hostile_seen |= int(attrs.get("visible_hostile_count", 0) or 0) > 0
            self.combat_seen |= bool(attrs.get("combat_active", False)) if attrs.get("combat_active_known") else False
            self.damage_dealt = max(self.damage_dealt, int(attrs.get("damage_dealt", self.damage_dealt) or 0))
            self.damage_received = max(self.damage_received, int(attrs.get("damage_received", self.damage_received) or 0))
        if event_name in {"world.player_died", "player.died", "body.death_detected"}:
            self.deaths += 1
        if "combat" in event_name and ("started" in event_name or "active" in event_name):
            self.combat_seen = True

    def verdict(self) -> tuple[bool, list[str]]:
        reasons: list[str] = []
        if not self.scenes:
            reasons.append("no_perception_frames")
        if not self.actions:
            reasons.append("no_body_actions")
        if not self.last_nav_scene and "move_to" not in self.actions:
            reasons.append("no_strategic_navigation_or_movement")
        if not self.hostile_seen:
            reasons.append("no_hostile_zone_seen")
        if not self.combat_seen and not any(k in self.actions for k in ("basic_attack", "attack", "use_action")):
            reasons.append("no_combat_started")
        if self.failures:
            reasons.append("model_or_runtime_failures_present")
        return not reasons, reasons


def print_status(run: Run, prefix: str = "") -> None:
    ok, reasons = run.verdict()
    status = "PASS" if ok else "WAIT/FAIL"
    print(
        f"{prefix}{status} run={run.run_id[:8]} character={run.character or '?'} "
        f"scene={run.last_scene or '?'} scenes={len(run.scenes)} "
        f"actions={dict(run.actions)} hostiles={run.hostile_seen} combat={run.combat_seen} "
        f"chat={run.chat_or_social} calls={run.model_calls} tokens={run.input_tokens}/{run.output_tokens} "
        f"cached={run.cached_tokens} reasoning={run.reasoning_tokens} cost=${run.cost_usd:.6f}"
    )
    if reasons:
        print(f"  reasons: {', '.join(reasons)}")
    if run.failures:
        print(f"  failures: {dict(run.failures)}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--run-id", help="Only consume this process_run_id")
    parser.add_argument("--watch", action="store_true", help="Follow the file until interrupted or success")
    parser.add_argument("--seconds", type=float, default=0, help="Bounded watch duration; 0 means no bound")
    parser.add_argument("--interval", type=float, default=5.0)
    args = parser.parse_args()
    run = Run(run_id=args.run_id or "")
    offset = 0
    deadline = time.time() + args.seconds if args.seconds else None
    while True:
        if not args.log.exists():
            print(f"waiting for log: {args.log}", file=sys.stderr)
            time.sleep(args.interval)
            continue
        for offset, event in json_lines(args.log, offset):
            if args.run_id and event.get("fields", {}).get("process_run_id") != args.run_id:
                continue
            run.ingest(event)
        print_status(run)
        passed, _ = run.verdict()
        if passed or not args.watch:
            return 0 if passed else 1
        if deadline and time.time() >= deadline:
            return 0 if passed else 1
        time.sleep(args.interval)


if __name__ == "__main__":
    raise SystemExit(main())
