#!/usr/bin/env python3
"""Run rule-indexed POSIX.1-2024 shell compliance cases."""

from __future__ import annotations

import argparse
import json
import os
import sys
from collections import Counter
from pathlib import Path

from cases import CASES
from catalog import (
    CatalogError,
    dispositions,
    load_overrides,
    load_rules,
    validate_registry,
)
from executor import host_capabilities, run_case
from model import Case, Observation, RuleResult
from report import (
    CASE_VERDICT_ORDER,
    RULE_VERDICT_ORDER,
    counts,
    report_dict,
    rollup_rules,
)


HARNESS_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = HARNESS_DIR.parents[1]
SPEC_DIR = PROJECT_ROOT / "posix" / "docs" / "spec"
OVERRIDES = HARNESS_DIR / "dispositions.json"


def _path(value: str) -> Path:
    return Path(value).expanduser().resolve()


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Judge a shell against executable expectations derived from the "
            "checked-in POSIX.1-2024 rule wording."
        )
    )
    parser.add_argument(
        "--shell",
        type=_path,
        default=(PROJECT_ROOT / "target" / "debug" / "dash").resolve(),
        help="shell under test (default: target/debug/dash)",
    )
    parser.add_argument(
        "--reference",
        type=_path,
        help="optional comparison shell; never used as the compliance oracle",
    )
    parser.add_argument(
        "--case",
        action="append",
        default=[],
        metavar="ID",
        help="run only this case; repeatable",
    )
    parser.add_argument(
        "--rule",
        action="append",
        default=[],
        metavar="ID",
        help="select cases touching this rule; repeatable",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="list the rule catalog and dispositions without executing",
    )
    parser.add_argument(
        "--format",
        choices=("text", "json"),
        default="text",
        help="report format",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="include passing cases in text output",
    )
    parser.add_argument(
        "--report-only",
        action="store_true",
        help="return success despite normative failures",
    )
    return parser


def _select_cases(cases: tuple[Case, ...], case_ids: list[str], rule_ids: list[str]) -> tuple[Case, ...]:
    known_cases = {case.id for case in cases}
    unknown_cases = sorted(set(case_ids) - known_cases)
    if unknown_cases:
        raise CatalogError(f"unknown case ids: {', '.join(unknown_cases)}")
    selected = cases
    if case_ids:
        wanted = set(case_ids)
        selected = tuple(case for case in selected if case.id in wanted)
    if rule_ids:
        wanted_rules = set(rule_ids)
        selected = tuple(case for case in selected if wanted_rules.intersection(case.rules))
    return selected


def _run(shell: Path, cases: tuple[Case, ...], capabilities: dict[str, bool]) -> tuple[Observation, ...]:
    return tuple(run_case(shell, case, capabilities) for case in cases)


def _summary_line(label: str, summary: dict[str, int]) -> str:
    body = " ".join(f"{name}={summary[name]}" for name in summary)
    return f"{label}: {body}"


def _print_observation(observation: Observation, reference: Observation | None = None) -> None:
    reference_text = f" reference={reference.verdict}" if reference else ""
    print(
        f"{observation.verdict:5} {observation.case}"
        f" status={observation.status}{reference_text}"
    )
    print(f"      rules: {', '.join(observation.rules)}")
    for reason in observation.reasons:
        print(f"      {reason}")
    if observation.verdict in {"FAIL", "ERROR"}:
        if observation.stdout:
            print(f"      stdout: {observation.stdout!r}")
        if observation.stderr:
            print(f"      stderr: {observation.stderr!r}")


def _print_text(
    *,
    shell: Path,
    reference: Path | None,
    observations: tuple[Observation, ...],
    reference_observations: tuple[Observation, ...],
    rule_results: tuple[RuleResult, ...],
    reference_rule_results: tuple[RuleResult, ...],
    capabilities: dict[str, bool],
    verbose: bool,
) -> None:
    print("POSIX.1-2024 rule harness")
    print(f"target: {shell}")
    if reference:
        print(f"reference: {reference} (comparison only)")
    print(
        "host options: "
        + " ".join(f"{name}={'yes' if active else 'no'}" for name, active in capabilities.items())
    )
    print(_summary_line("cases", counts(observations, "verdict", CASE_VERDICT_ORDER)))
    print(_summary_line("rules", counts(rule_results, "verdict", RULE_VERDICT_ORDER)))
    dispositions_summary = Counter(item.disposition for item in rule_results)
    print(
        "dispositions: "
        + " ".join(
            f"{name}={dispositions_summary[name]}"
            for name in ("automatic", "manual", "conditional", "not-applicable", "pending")
        )
    )
    if reference:
        print(
            _summary_line(
                "reference cases",
                counts(reference_observations, "verdict", CASE_VERDICT_ORDER),
            )
        )
        print(
            _summary_line(
                "reference rules",
                counts(reference_rule_results, "verdict", RULE_VERDICT_ORDER),
            )
        )

    reference_by_case = {item.case: item for item in reference_observations}
    visible = observations if verbose else tuple(
        item for item in observations if item.verdict != "PASS"
    )
    if visible:
        print()
        for observation in visible:
            _print_observation(observation, reference_by_case.get(observation.case))

    if reference:
        differentials = []
        for observation in observations:
            other = reference_by_case.get(observation.case)
            if other is None:
                continue
            if observation.verdict != other.verdict or observation.status != other.status:
                differentials.append((observation, other))
        if differentials:
            print("\ndifferentials:")
            for target, other in differentials:
                print(
                    f"  {target.case}: target={target.verdict}/{target.status} "
                    f"reference={other.verdict}/{other.status}"
                )


def _list_catalog(
    rules: dict[str, object],
    rule_dispositions: dict[str, str],
    cases: tuple[Case, ...],
    selected_rules: set[str],
) -> None:
    cases_by_rule: dict[str, list[str]] = {}
    for case in cases:
        for rule_id in case.rules:
            cases_by_rule.setdefault(rule_id, []).append(case.id)
    for rule_id, rule in sorted(rules.items()):
        if selected_rules and rule_id not in selected_rules:
            continue
        condition_text = ",".join(rule.conditions) or "-"
        case_text = ",".join(cases_by_rule.get(rule_id, [])) or "-"
        source = rule.source.relative_to(PROJECT_ROOT)
        print(
            f"{rule_dispositions[rule_id]:14} {rule.verb:3} {rule_id} "
            f"conditions={condition_text} cases={case_text} {source}:{rule.line}"
        )


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    try:
        rules = load_rules(SPEC_DIR)
        cases = validate_registry(rules, CASES)
        overrides = load_overrides(OVERRIDES)
        rule_dispositions = dispositions(rules, cases, overrides)
        unknown_rules = sorted(set(args.rule) - rules.keys())
        if unknown_rules:
            raise CatalogError(f"unknown rule ids: {', '.join(unknown_rules)}")
        selected = _select_cases(cases, args.case, args.rule)
    except (CatalogError, OSError, json.JSONDecodeError) as error:
        print(f"posix-harness: {error}", file=sys.stderr)
        return 2

    if args.list:
        _list_catalog(rules, rule_dispositions, cases, set(args.rule))
        return 0

    if not args.shell.is_file() or not os.access(args.shell, os.X_OK):
        print(f"posix-harness: shell not executable: {args.shell}", file=sys.stderr)
        return 2
    if args.reference and (
        not args.reference.is_file() or not os.access(args.reference, os.X_OK)
    ):
        print(
            f"posix-harness: reference shell not executable: {args.reference}",
            file=sys.stderr,
        )
        return 2

    capabilities = host_capabilities()
    observations = _run(args.shell, selected, capabilities)
    reference_observations = (
        _run(args.reference, selected, capabilities) if args.reference else ()
    )
    rule_results = rollup_rules(
        rules,
        selected,
        observations,
        rule_dispositions,
        capabilities,
        PROJECT_ROOT,
    )
    reference_rule_results = (
        rollup_rules(
            rules,
            selected,
            reference_observations,
            rule_dispositions,
            capabilities,
            PROJECT_ROOT,
        )
        if args.reference
        else ()
    )

    if args.format == "json":
        print(
            json.dumps(
                report_dict(
                    shell=args.shell,
                    reference=args.reference,
                    observations=observations,
                    reference_observations=reference_observations,
                    rule_results=rule_results,
                    reference_rule_results=reference_rule_results,
                    capabilities=capabilities,
                ),
                indent=2,
                sort_keys=True,
            )
        )
    else:
        _print_text(
            shell=args.shell,
            reference=args.reference,
            observations=observations,
            reference_observations=reference_observations,
            rule_results=rule_results,
            reference_rule_results=reference_rule_results,
            capabilities=capabilities,
            verbose=args.verbose,
        )

    failed = any(item.verdict in {"FAIL", "ERROR"} for item in observations)
    return 0 if args.report_only or not failed else 1


if __name__ == "__main__":
    raise SystemExit(main())
