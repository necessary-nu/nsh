"""Rule rollups and stable text/JSON report structures."""

from __future__ import annotations

from collections import Counter, defaultdict
from pathlib import Path

from model import Case, Disposition, Observation, Rule, RuleResult


RULE_VERDICT_ORDER = ("PASS", "FAIL", "SKIP", "MANUAL", "UNTESTED")
CASE_VERDICT_ORDER = ("PASS", "FAIL", "SKIP", "ERROR")
DISPOSITION_ORDER = (
    "automatic",
    "manual",
    "conditional",
    "not-applicable",
    "pending",
)


def rollup_rules(
    rules: dict[str, Rule],
    cases: tuple[Case, ...],
    observations: tuple[Observation, ...],
    rule_dispositions: dict[str, Disposition],
    capabilities: dict[str, bool],
    project_root: Path,
) -> tuple[RuleResult, ...]:
    cases_by_rule: dict[str, list[str]] = defaultdict(list)
    observations_by_rule: dict[str, list[Observation]] = defaultdict(list)
    for case in cases:
        for rule_id in case.rules:
            cases_by_rule[rule_id].append(case.id)
    for observation in observations:
        for rule_id in observation.rules:
            observations_by_rule[rule_id].append(observation)

    results: list[RuleResult] = []
    for rule_id, rule in sorted(rules.items()):
        disposition = rule_dispositions[rule_id]
        observed = observations_by_rule.get(rule_id, [])
        if any(item.verdict in {"FAIL", "ERROR"} for item in observed):
            verdict = "FAIL"
        elif any(item.verdict == "PASS" for item in observed):
            verdict = "PASS"
        elif observed and all(item.verdict == "SKIP" for item in observed):
            verdict = "SKIP"
        elif disposition == "manual":
            verdict = "MANUAL"
        elif disposition == "not-applicable":
            verdict = "SKIP"
        elif disposition == "conditional" and any(
            not capabilities.get(code, False) for code in rule.conditions if code != "OB"
        ):
            verdict = "SKIP"
        else:
            verdict = "UNTESTED"

        try:
            source = str(rule.source.relative_to(project_root))
        except ValueError:
            source = str(rule.source)
        results.append(
            RuleResult(
                id=rule.id,
                verb=rule.verb,
                disposition=disposition,
                verdict=verdict,
                cases=tuple(cases_by_rule.get(rule_id, [])),
                conditions=rule.conditions,
                source=source,
                line=rule.line,
            )
        )
    return tuple(results)


def counts(values: tuple[object, ...], attribute: str, order: tuple[str, ...]) -> dict[str, int]:
    found = Counter(getattr(value, attribute) for value in values)
    return {name: found[name] for name in order}


def report_dict(
    *,
    shell: Path,
    reference: Path | None,
    observations: tuple[Observation, ...],
    reference_observations: tuple[Observation, ...],
    rule_results: tuple[RuleResult, ...],
    reference_rule_results: tuple[RuleResult, ...],
    capabilities: dict[str, bool],
) -> dict[str, object]:
    reference_by_case = {item.case: item for item in reference_observations}
    differentials = []
    for item in observations:
        other = reference_by_case.get(item.case)
        if other is None:
            continue
        if item.verdict != other.verdict or item.status != other.status:
            differentials.append(
                {
                    "case": item.case,
                    "target_verdict": item.verdict,
                    "reference_verdict": other.verdict,
                    "target_status": item.status,
                    "reference_status": other.status,
                    "target_stdout": item.stdout,
                    "reference_stdout": other.stdout,
                    "target_stderr": item.stderr,
                    "reference_stderr": other.stderr,
                }
            )
    return {
        "schema_version": 1,
        "target": str(shell),
        "reference": str(reference) if reference else None,
        "capabilities": capabilities,
        "summary": {
            "cases": counts(observations, "verdict", CASE_VERDICT_ORDER),
            "rules": counts(rule_results, "verdict", RULE_VERDICT_ORDER),
            "dispositions": counts(
                rule_results, "disposition", DISPOSITION_ORDER
            ),
        },
        "reference_summary": {
            "cases": counts(reference_observations, "verdict", CASE_VERDICT_ORDER),
            "rules": counts(reference_rule_results, "verdict", RULE_VERDICT_ORDER),
        }
        if reference
        else None,
        "observations": [item.to_dict() for item in observations],
        "reference_observations": [item.to_dict() for item in reference_observations],
        "rules": [item.__dict__ for item in rule_results],
        "reference_rules": [item.__dict__ for item in reference_rule_results],
        "differentials": differentials,
    }
