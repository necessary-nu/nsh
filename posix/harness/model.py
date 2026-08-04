"""Data model shared by the POSIX rule catalog and executable harness."""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Literal, TypeAlias


Disposition: TypeAlias = Literal[
    "automatic", "manual", "conditional", "not-applicable", "pending"
]
StatusExpectation: TypeAlias = int | Literal["nonzero", "any"]
Verdict: TypeAlias = Literal["PASS", "FAIL", "SKIP", "ERROR"]
ExecutionMode: TypeAlias = Literal["command", "stdin", "interactive"]


@dataclass(frozen=True)
class Rule:
    """One rule extracted from the vendored POSIX nspec corpus."""

    id: str
    verb: str
    version: int
    body: str
    source: Path
    line: int
    conditions: tuple[str, ...] = ()

    @property
    def annotation(self) -> str:
        return f"[spec:posix:{self.verb}:{self.id}]"


@dataclass(frozen=True)
class FileFixture:
    """A file installed in an isolated case directory before execution."""

    content: str
    mode: int = 0o644


@dataclass(frozen=True)
class Case:
    """An executable expectation derived from one or more POSIX rules."""

    id: str
    rules: tuple[str, ...]
    script: str
    stdout: str | None = ""
    stderr: str | None = None
    status: StatusExpectation = 0
    stdout_contains: tuple[str, ...] = ()
    stderr_contains: tuple[str, ...] = ()
    stdout_excludes: tuple[str, ...] = ()
    stderr_excludes: tuple[str, ...] = ()
    args: tuple[str, ...] = ()
    files: dict[str, FileFixture] = field(default_factory=dict)
    environment: dict[str, str] = field(default_factory=dict)
    stdin: str | None = None
    mode: ExecutionMode = "command"
    shell_options: tuple[str, ...] = ()
    timeout: float = 5.0
    requires: tuple[str, ...] = ()
    # Bind a private writable /tmp into the namespace. Off by default: a
    # bind over /tmp hides everything beneath it, and both the case root
    # and (depending on how it was built) the shell under test can live
    # there. Only cases that need a writable /tmp -- dash's `fc` editor
    # path uses a compile-time _PATH_TMP rather than $TMPDIR -- turn it on.
    writable_tmp: bool = False


@dataclass
class Observation:
    """The complete observable result of running one case against one shell."""

    case: str
    rules: tuple[str, ...]
    verdict: Verdict
    status: int | None
    stdout: str
    stderr: str
    reasons: list[str]
    duration_ms: int
    skipped_by: tuple[str, ...] = ()

    def to_dict(self) -> dict[str, object]:
        return asdict(self)


@dataclass(frozen=True)
class RuleResult:
    """Rule-level rollup used by text and JSON reports."""

    id: str
    verb: str
    disposition: Disposition
    verdict: Literal["PASS", "FAIL", "SKIP", "MANUAL", "UNTESTED"]
    cases: tuple[str, ...]
    conditions: tuple[str, ...]
    source: str
    line: int
