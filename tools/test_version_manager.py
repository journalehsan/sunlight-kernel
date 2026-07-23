#!/usr/bin/env python3

import contextlib
import datetime
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import version_manager as vm


UTC = datetime.UTC


class TimeIdentityTests(unittest.TestCase):
    def identity(self, date: str, patch: int = 0) -> dict:
        return vm.calculate_time_identity(datetime.date.fromisoformat(date), patch)

    def test_epoch_boundary(self) -> None:
        identity = self.identity("2026-06-06", 7)
        self.assertEqual(identity["version"], "0.1.7-alpha.1")
        self.assertFalse(identity["stable_boundary"])

    def test_every_prerelease_threshold(self) -> None:
        cases = {
            "2026-06-06": "alpha.1",
            "2026-09-06": "alpha.2",
            "2026-12-06": "alpha.3",
            "2027-06-06": "beta.1",
            "2027-09-06": "beta.2",
            "2027-12-06": "rc.1",
            "2028-03-06": "rc.2",
            "2028-12-05": "rc.2",
        }
        for date, expected in cases.items():
            with self.subTest(date=date):
                self.assertEqual(self.identity(date)["prerelease"], expected)

    def test_december_to_january_does_not_decrease_minor(self) -> None:
        december = self.identity("2026-12-31", 3)["version"]
        january = self.identity("2027-01-01", 3)["version"]
        self.assertEqual(december, "0.2.3-alpha.3")
        self.assertEqual(january, december)

    def test_every_pre_mercury_six_month_minor_transition(self) -> None:
        transitions = [
            ("2026-12-05", "0.1.0-alpha.2", "2026-12-06", "0.2.0-alpha.3"),
            ("2027-06-05", "0.2.0-alpha.3", "2027-06-06", "0.3.0-beta.1"),
            ("2027-12-05", "0.3.0-beta.2", "2027-12-06", "0.4.0-rc.1"),
            ("2028-06-05", "0.4.0-rc.2", "2028-06-06", "0.5.0-rc.2"),
        ]
        for before_date, before_version, after_date, after_version in transitions:
            with self.subTest(after_date=after_date):
                self.assertEqual(self.identity(before_date)["version"], before_version)
                self.assertEqual(self.identity(after_date)["version"], after_version)
                self.assertGreater(
                    vm.compare_semver(after_version, before_version),
                    0,
                )

    def test_every_generation_six_month_minor_transition(self) -> None:
        generation_starts = [vm.PROJECT_EPOCH]
        generation_starts.extend(vm._planetary_boundary(major) for major in range(1, 8))
        for major, generation_start in enumerate(generation_starts):
            for minor in range(2, 6):
                transition = vm._add_months(generation_start, (minor - 1) * 6)
                before = vm.calculate_time_identity(
                    transition - datetime.timedelta(days=1),
                    11,
                )
                after = vm.calculate_time_identity(transition, 11)
                with self.subTest(major=major, minor=minor):
                    self.assertEqual(after["major"], major)
                    self.assertEqual(after["minor"], minor)
                    self.assertGreater(
                        vm.compare_semver(after["version"], before["version"]),
                        0,
                    )

    def test_mercury_boundary_and_post_release_ordering(self) -> None:
        before = self.identity("2028-12-05", 41)
        mercury = self.identity("2028-12-06", 41)
        after = self.identity("2028-12-07", 41)
        self.assertEqual(before["version"], "0.5.41-rc.2")
        self.assertEqual(mercury["version"], "1.0.0")
        self.assertEqual(mercury["release_name"], "Mercury")
        self.assertEqual(after["version"], "1.1.41-alpha.1")
        self.assertGreater(vm.compare_semver(mercury["version"], before["version"]), 0)
        self.assertGreater(vm.compare_semver(after["version"], mercury["version"]), 0)

    def test_planetary_stable_identities(self) -> None:
        for major, name in enumerate(vm.PLANET_NAMES, start=1):
            boundary = vm._planetary_boundary(major)
            with self.subTest(major=major, name=name):
                identity = vm.calculate_time_identity(boundary, 999)
                self.assertEqual(identity["version"], f"{major}.0.0")
                self.assertEqual(identity["release_name"], name)

    def test_venus_and_later_planet(self) -> None:
        self.assertEqual(self.identity("2031-06-06")["version"], "2.0.0")
        self.assertEqual(self.identity("2036-06-06")["version"], "4.0.0")

    def test_neptune(self) -> None:
        identity = self.identity("2046-06-06")
        self.assertEqual(identity["version"], "8.0.0")
        self.assertEqual(identity["release_name"], "Neptune")

    def test_calendar_month_and_leap_year_behavior(self) -> None:
        self.assertEqual(
            vm._add_months(datetime.date(2024, 1, 31), 1),
            datetime.date(2024, 2, 29),
        )
        self.assertEqual(
            vm._add_months(datetime.date(2025, 1, 31), 1),
            datetime.date(2025, 2, 28),
        )
        self.assertEqual(vm._months_since_epoch(datetime.date(2028, 2, 5)), 19)
        self.assertEqual(vm._months_since_epoch(datetime.date(2028, 2, 6)), 20)

    def test_semver_never_decreases_through_neptune(self) -> None:
        date = vm.PROJECT_EPOCH
        end = vm._planetary_boundary(8)
        previous = vm.calculate_time_identity(date, 23)["version"]
        while date < end:
            date += datetime.timedelta(days=1)
            current = vm.calculate_time_identity(date, 23)["version"]
            self.assertGreaterEqual(
                vm.compare_semver(current, previous),
                0,
                f"{date.isoformat()}: {previous} -> {current}",
            )
            previous = current


class StateTests(unittest.TestCase):
    def test_weighted_baseline_migration_preserves_version_floor(self) -> None:
        legacy = {
            "last_known_lines": 240835,
            "accumulated_patch": 2073,
            "last_version": "0.2.2073",
            "last_total_lines": 240832,
            "history": [],
            "_migration_source": "legacy-raw-v0",
        }
        state, diagnostic = vm._normalise_state(
            legacy,
            current_weighted=500_000.0,
            canonical_date=datetime.date(2026, 7, 23),
        )
        identity = vm.calculate_time_identity(
            datetime.date(2026, 7, 23),
            state["accumulated_patch"],
            minor_offset_major=state["minor_offset_major"],
            minor_offset=state["minor_offset"],
        )
        self.assertEqual(state["last_known_weighted_lines"], 500_000.0)
        self.assertEqual(state["accumulated_patch"], 2074)
        self.assertEqual(state["minor_offset_major"], 0)
        self.assertEqual(state["minor_offset"], 1)
        self.assertGreater(vm.compare_semver(identity["version"], "0.2.2073"), 0)
        self.assertIn("preserved monotonic version floor", diagnostic)

    def test_migration_accepts_equal_stable_boundary(self) -> None:
        legacy = {
            "last_known_lines": 100,
            "accumulated_patch": 9,
            "last_version": "1.0.0",
            "last_total_lines": 100,
            "history": [],
            "_migration_source": "legacy-raw-v0",
        }
        state, _ = vm._normalise_state(
            legacy,
            current_weighted=200.0,
            canonical_date=datetime.date(2028, 12, 6),
        )
        self.assertEqual(state["accumulated_patch"], 9)

    def test_invalid_json_state_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            Path(temporary, vm.STATE_FILE).write_text("{", encoding="utf-8")
            with self.assertRaisesRegex(vm.VersionStateError, "valid JSON"):
                vm.load_state(temporary)

    def test_incompatible_algorithm_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            Path(temporary, vm.STATE_FILE).write_text(
                json.dumps(
                    {
                        "version_algorithm": "weighted-v2",
                        "last_known_weighted_lines": 10,
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(vm.VersionStateError, "explicit migration"):
                vm.load_state(temporary)

    def test_future_state_schema_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            Path(temporary, vm.STATE_FILE).write_text(
                json.dumps(
                    {
                        "state_schema": vm.STATE_SCHEMA_VERSION + 1,
                        "version_algorithm": vm.ALGORITHM_VERSION,
                        "last_known_weighted_lines": 10,
                        "accumulated_patch": 1,
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(vm.VersionStateError, "schema"):
                vm.load_state(temporary)

    def test_stale_canonical_time_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            state = vm._fresh_state()
            state.update(
                {
                    "last_known_weighted_lines": 0,
                    "last_total_weighted_lines": 0,
                    "last_canonical_epoch": 2_000_000_000,
                }
            )
            Path(temporary, vm.STATE_FILE).write_text(
                json.dumps(state),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(vm.VersionStateError, "stale"):
                vm.calculate_version(
                    temporary,
                    canonical_datetime=datetime.datetime(
                        2026, 6, 6, tzinfo=UTC
                    ),
                )

    def test_dry_run_does_not_mutate_state_or_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest = Path(temporary, "Cargo.toml")
            original = '[package]\nname = "example"\nversion = "9.9.9"\n'
            manifest.write_text(original, encoding="utf-8")
            epoch = str(int(datetime.datetime(2026, 6, 6, tzinfo=UTC).timestamp()))
            with mock.patch.dict(os.environ, {"SOURCE_DATE_EPOCH": epoch}):
                with contextlib.redirect_stdout(io.StringIO()):
                    vm.main(["--dry-run", temporary])
            self.assertEqual(manifest.read_text(encoding="utf-8"), original)
            self.assertFalse(Path(temporary, vm.STATE_FILE).exists())


class ReproducibilityAndWeightTests(unittest.TestCase):
    def test_source_date_epoch_overrides_head(self) -> None:
        epoch = int(datetime.datetime(2028, 12, 6, tzinfo=UTC).timestamp())
        resolved, source = vm.resolve_canonical_datetime(
            "/does/not/need/git",
            {"SOURCE_DATE_EPOCH": str(epoch)},
        )
        self.assertEqual(resolved, datetime.datetime(2028, 12, 6, tzinfo=UTC))
        self.assertEqual(source, "SOURCE_DATE_EPOCH")

    def test_structurally_recognised_application_gets_application_weight(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            crate = Path(temporary, "sunlight-new-app")
            Path(crate, "src").mkdir(parents=True)
            Path(crate, "Cargo.toml").write_text(
                '[package]\nname = "sunlight-new-app"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            Path(crate, "src", "main.rs").write_text("fn main() {}\n", encoding="utf-8")
            self.assertEqual(
                vm._get_weight("sunlight-new-app/src/main.rs", temporary),
                1.0,
            )


if __name__ == "__main__":
    unittest.main()
