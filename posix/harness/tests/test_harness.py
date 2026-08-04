"""Self-tests for catalog extraction, matching, and rule rollups."""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


HARNESS_DIR = Path(__file__).resolve().parents[1]
PROJECT_ROOT = HARNESS_DIR.parents[1]
sys.path.insert(0, str(HARNESS_DIR))

from cases import CASES  # noqa: E402
from catalog import dispositions, load_overrides, load_rules, validate_registry  # noqa: E402
from executor import run_case  # noqa: E402
from model import Case, Observation  # noqa: E402
from report import report_dict, rollup_rules  # noqa: E402


class CatalogTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.rules = load_rules(PROJECT_ROOT / "posix" / "docs" / "spec")
        cls.cases = validate_registry(cls.rules, CASES)
        cls.overrides = load_overrides(HARNESS_DIR / "dispositions.json")
        cls.dispositions = dispositions(cls.rules, cls.cases, cls.overrides)

    def test_extracts_complete_unique_corpus(self) -> None:
        self.assertEqual(len(self.rules), 1130)
        self.assertEqual(len(self.rules), len(set(self.rules)))
        rule = self.rules["builtin.set.opt-u-nounset"]
        self.assertEqual(rule.verb, "req")
        self.assertIn("arithmetic expansion", rule.body)
        self.assertGreater(rule.line, 0)
        theorem = self.rules["builtin.read.single-var-unsplit"]
        self.assertEqual(theorem.verb, "thm")

    def test_every_rule_has_a_disposition(self) -> None:
        self.assertEqual(set(self.dispositions), set(self.rules))
        self.assertEqual(
            sum(value == "automatic" for value in self.dispositions.values()),
            len({rule for case in self.cases for rule in case.rules}),
        )

    def test_seed_registry_is_normative_and_unique(self) -> None:
        self.assertEqual(len(self.cases), 69)
        self.assertEqual(len({case.id for case in self.cases}), len(self.cases))
        self.assertTrue(all(case.rules for case in self.cases))


class ExecutorTests(unittest.TestCase):
    def test_exact_expectation_passes(self) -> None:
        observation = run_case(
            Path("/bin/sh"),
            Case(
                id="self-pass",
                rules=("quote.single-quotes",),
                script="printf '%s\\n' 'ok'\n",
                stdout="ok\n",
            ),
            {"UP": True, "XSI": True, "OB": True},
        )
        self.assertEqual(observation.verdict, "PASS")
        self.assertEqual(observation.status, 0)

    def test_mismatch_is_explained(self) -> None:
        observation = run_case(
            Path("/bin/sh"),
            Case(
                id="self-fail",
                rules=("quote.single-quotes",),
                script="printf actual\n",
                stdout="expected\n",
                status=7,
            ),
            {"UP": True, "XSI": True, "OB": True},
        )
        self.assertEqual(observation.verdict, "FAIL")
        self.assertIn("expected status 7, got 0", observation.reasons)
        self.assertTrue(any(reason.startswith("stdout differs") for reason in observation.reasons))

    def test_nested_sh_resolves_to_target(self) -> None:
        observation = run_case(
            Path("/bin/sh"),
            Case(
                id="self-shim",
                rules=("sh.option-c",),
                script="sh -c 'printf nested'\n",
                stdout="nested",
            ),
            {"UP": True, "XSI": True, "OB": True},
        )
        self.assertEqual(observation.verdict, "PASS")

    def test_script_can_be_supplied_on_standard_input(self) -> None:
        observation = run_case(
            Path("/bin/sh"),
            Case(
                id="self-stdin",
                rules=("sh.option-s",),
                script="printf '%s\\n' stdin-script\n",
                stdout="stdin-script\n",
                mode="stdin",
            ),
            {"UP": True, "XSI": True, "OB": True},
        )
        self.assertEqual(observation.verdict, "PASS", observation.reasons)

    def test_interactive_mode_uses_a_controlling_terminal(self) -> None:
        observation = run_case(
            Path("/bin/sh"),
            Case(
                id="self-interactive",
                rules=("sh.option-i",),
                script="printf '%s\\n' interactive-session\nexit\n",
                stdout="interactive-session\n",
                environment={"PS1": "", "PS2": ""},
                mode="interactive",
            ),
            {"UP": True, "XSI": True, "OB": True},
        )
        self.assertEqual(observation.verdict, "PASS", observation.reasons)

    def test_environment_placeholders_are_expanded(self) -> None:
        observation = run_case(
            Path("/bin/sh"),
            Case(
                id="self-environment",
                rules=("param.home",),
                script="printf '%s\\n' \"$AUDIT_HOME\"\n",
                stdout="{HOME}\n",
                environment={"AUDIT_HOME": "{HOME}"},
            ),
            {"UP": True, "XSI": True, "OB": True},
        )
        self.assertEqual(observation.verdict, "PASS", observation.reasons)

    def test_missing_capability_skips(self) -> None:
        observation = run_case(
            Path("/bin/sh"),
            Case(
                id="self-skip",
                rules=("edit.history-list",),
                script="exit 99\n",
                requires=("UP",),
            ),
            {"UP": False, "XSI": False, "OB": True},
        )
        self.assertEqual(observation.verdict, "SKIP")
        self.assertIsNone(observation.status)

    def test_timeout_terminates_the_case_process_group(self) -> None:
        observation = run_case(
            Path("/bin/sh"),
            Case(
                id="self-timeout",
                rules=("cmd.loop-while-until",),
                script="while :; do :; done\n",
                status="any",
                timeout=0.05,
            ),
            {"UP": True, "XSI": True, "OB": True},
        )
        self.assertEqual(observation.verdict, "FAIL")
        self.assertEqual(observation.status, 124)
        self.assertTrue(any(reason.startswith("timed out") for reason in observation.reasons))


class ReportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.rules = load_rules(PROJECT_ROOT / "posix" / "docs" / "spec")
        cls.cases = validate_registry(cls.rules, CASES)
        cls.rule_dispositions = dispositions(
            cls.rules,
            cls.cases,
            load_overrides(HARNESS_DIR / "dispositions.json"),
        )

    def test_failure_overrides_passing_case_for_same_rule(self) -> None:
        selected = (
            Case("one", ("builtin.trap.action-executed-as-eval",), ":"),
            Case("two", ("builtin.trap.action-executed-as-eval",), ":"),
        )
        observations = (
            Observation(
                "one",
                selected[0].rules,
                "PASS",
                0,
                "",
                "",
                [],
                1,
            ),
            Observation(
                "two",
                selected[1].rules,
                "FAIL",
                0,
                "",
                "",
                ["counterexample"],
                1,
            ),
        )
        results = rollup_rules(
            self.rules,
            selected,
            observations,
            self.rule_dispositions,
            {"UP": True, "XSI": True, "OB": True},
            PROJECT_ROOT,
        )
        result = next(
            item for item in results if item.id == "builtin.trap.action-executed-as-eval"
        )
        self.assertEqual(result.verdict, "FAIL")

    def test_json_report_is_serializable(self) -> None:
        report = report_dict(
            shell=Path("/bin/sh"),
            reference=None,
            observations=(),
            reference_observations=(),
            rule_results=(),
            reference_rule_results=(),
            capabilities={"UP": True, "XSI": True, "OB": True},
        )
        encoded = json.dumps(report)
        self.assertIn('"schema_version": 1', encoded)
        self.assertEqual(
            report["summary"]["dispositions"],
            {
                "automatic": 0,
                "manual": 0,
                "conditional": 0,
                "not-applicable": 0,
                "pending": 0,
            },
        )


if __name__ == "__main__":
    unittest.main()
