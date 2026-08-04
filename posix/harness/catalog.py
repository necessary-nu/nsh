"""Extract the compliance catalog from the checked-in POSIX rule wording."""

from __future__ import annotations

import json
import re
from collections.abc import Iterable
from pathlib import Path

from model import Case, Disposition, Rule


MARKER = re.compile(
    r"^>\s*\[spec:posix:(def|syn|sem|req|thm):([a-z0-9][a-z0-9_.-]*)(?:\+(\d+))?\]\s*$"
)
VALID_DISPOSITIONS: frozenset[str] = frozenset(
    {"automatic", "manual", "conditional", "not-applicable", "pending"}
)


class CatalogError(ValueError):
    """Raised when the corpus, overrides, or executable registry disagree."""


def _conditions(body: str) -> tuple[str, ...]:
    return tuple(code for code in ("UP", "XSI", "OB") if f"[{code}]" in body)


def load_rules(spec_dir: Path) -> dict[str, Rule]:
    """Parse every nspec rule block from ``spec_dir``.

    The body is read directly from the contiguous Markdown blockquote following
    the marker. This deliberately avoids a generated intermediate schema: the
    normative wording checked into the repository remains the source of truth.
    """

    rules: dict[str, Rule] = {}
    for path in sorted(spec_dir.glob("*.md")):
        lines = path.read_text(encoding="utf-8").splitlines()
        index = 0
        while index < len(lines):
            match = MARKER.match(lines[index])
            if match is None:
                index += 1
                continue

            verb, rule_id, version_text = match.groups()
            body_lines: list[str] = []
            cursor = index + 1
            while cursor < len(lines) and lines[cursor].startswith(">"):
                text = lines[cursor][1:]
                if text.startswith(" "):
                    text = text[1:]
                body_lines.append(text)
                cursor += 1
            body = "\n".join(body_lines).strip()
            if not body:
                raise CatalogError(f"{path}:{index + 1}: rule {rule_id} has no body")
            if rule_id in rules:
                previous = rules[rule_id]
                raise CatalogError(
                    f"duplicate rule {rule_id}: {previous.source}:{previous.line} and "
                    f"{path}:{index + 1}"
                )
            rules[rule_id] = Rule(
                id=rule_id,
                verb=verb,
                version=int(version_text or 0),
                body=body,
                source=path,
                line=index + 1,
                conditions=_conditions(body),
            )
            index = cursor
    if not rules:
        raise CatalogError(f"no POSIX rules found under {spec_dir}")
    return rules


def load_overrides(path: Path) -> dict[str, tuple[Disposition, str]]:
    """Load reviewed non-automatic dispositions from a small JSON sidecar."""

    raw = json.loads(path.read_text(encoding="utf-8"))
    if raw.get("version") != 1 or not isinstance(raw.get("rules"), dict):
        raise CatalogError(f"{path}: expected version 1 and a rules object")
    result: dict[str, tuple[Disposition, str]] = {}
    for rule_id, entry in raw["rules"].items():
        if not isinstance(entry, dict):
            raise CatalogError(f"{path}: override for {rule_id} must be an object")
        disposition = entry.get("disposition")
        reason = entry.get("reason")
        if disposition not in VALID_DISPOSITIONS:
            raise CatalogError(f"{path}: invalid disposition for {rule_id}: {disposition!r}")
        if not isinstance(reason, str) or not reason.strip():
            raise CatalogError(f"{path}: override for {rule_id} needs a reason")
        result[rule_id] = (disposition, reason.strip())
    return result


def validate_registry(rules: dict[str, Rule], cases: Iterable[Case]) -> tuple[Case, ...]:
    """Reject stale rule ids, duplicate case ids, and malformed fixtures early."""

    materialized = tuple(cases)
    seen: set[str] = set()
    for case in materialized:
        if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", case.id):
            raise CatalogError(f"invalid case id {case.id!r}")
        if case.id in seen:
            raise CatalogError(f"duplicate case id {case.id}")
        seen.add(case.id)
        if not case.rules:
            raise CatalogError(f"case {case.id} has no normative rule")
        if case.mode not in {"command", "stdin", "interactive"}:
            raise CatalogError(f"case {case.id} has invalid execution mode {case.mode!r}")
        if case.timeout <= 0:
            raise CatalogError(f"case {case.id} has non-positive timeout")
        unknown_options = sorted(set(case.requires) - {"UP", "XSI", "OB"})
        if unknown_options:
            raise CatalogError(
                f"case {case.id} requires unknown options: {', '.join(unknown_options)}"
            )
        for name in case.environment:
            if not name or "=" in name or "\0" in name:
                raise CatalogError(f"case {case.id} has invalid environment name {name!r}")
        unknown = sorted(set(case.rules) - rules.keys())
        if unknown:
            raise CatalogError(f"case {case.id} references unknown rules: {', '.join(unknown)}")
        for relative in case.files:
            fixture = Path(relative)
            if fixture.is_absolute() or ".." in fixture.parts:
                raise CatalogError(f"case {case.id} has unsafe fixture path {relative!r}")
    return materialized


def dispositions(
    rules: dict[str, Rule],
    cases: Iterable[Case],
    overrides: dict[str, tuple[Disposition, str]],
) -> dict[str, Disposition]:
    """Give every corpus rule an explicit current disposition."""

    unknown_overrides = sorted(set(overrides) - rules.keys())
    if unknown_overrides:
        raise CatalogError(f"overrides reference unknown rules: {', '.join(unknown_overrides)}")
    automatic = {rule for case in cases for rule in case.rules}
    result: dict[str, Disposition] = {}
    for rule_id, rule in rules.items():
        if rule_id in automatic:
            result[rule_id] = "automatic"
        elif rule_id in overrides:
            result[rule_id] = overrides[rule_id][0]
        elif rule.verb == "def":
            result[rule_id] = "manual"
        elif "UP" in rule.conditions or "XSI" in rule.conditions:
            result[rule_id] = "conditional"
        else:
            result[rule_id] = "pending"
    return result
