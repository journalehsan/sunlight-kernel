#!/usr/bin/env python3
"""
Auto Version Manager for SunlightOS
====================================
Version format: MAJOR.MINOR.PATCH[-PRERELEASE]

MAJOR:    Completed 30-month planetary generations.  Exact generation boundaries
          automatically produce MAJOR.0.0 stable releases.
MINOR:    One-based six-month index within the active generation.  It resets to
          1 only after MAJOR increases; stable boundaries use MINOR 0.
PATCH:    Accumulated weighted line changes (weighted_lines / LINES_PER_PATCH).
PRERELEASE: Repository-time maturity stage within the active 30-month cycle.

Timeline within each 30-month cycle (from cycle start):
  0 –  3 months  →  alpha.1
  3 –  6 months  →  alpha.2
  6 – 12 months  →  alpha.3   (first year is alpha)
 12 – 15 months  →  beta.1
 15 – 18 months  →  beta.2
 18 – 21 months  →  rc.1
 21 – 30 months  →  rc.2
 Exact boundary  →  stable MAJOR.0.0 planetary release

Line weights (used instead of raw line counts for PATCH calculation):
  kernel/                         4.0   (microkernel, memory, scheduler)
  ipc/                            3.0   (IPC types and ABI)
  drivers/                        3.0   (driver framework)
  compat-linux/                   2.5   (Linux compatibility / Helios)
  services/                       2.0   (system services)
  sunlight-fs/, sunlight-fat/     1.8   (filesystem infrastructure)
  sunlight-block/                 1.8
  sunlight-virtio/                1.8
  sunlight-net/, sunlight-tls/    1.8   (networking infrastructure)
  sunlight-http/                  1.8
  sunlight-net-utils/             1.8
  sunlight-fetch/                 1.8
  sunlight-devices/               1.8
  sunlight-libc/                  1.5   (core userland)
  sunlight-elf/                   1.5
  sunshell/                       1.5
  sunlight-tty/, sunlight-tui/    1.5
  sunlight-ui/                    1.5
  sunlight-utils/                 1.5
  sunlight-tz/                    1.5
  sunlight-kv/, sunlight-kvctl/   1.5
  sunlight-locale/                1.5
  sunlight-telemetry/             1.5
  sunlight-shell-appstate/        1.5
  sunlight-top/                   1.5
  sunlight-dialogs/               1.5
  sunlight-edit/                  1.5
  sunlight-clipman/               1.5
  cpu-utils/                      1.5
  sunlight-calculator/            1.0   (applications)
  sunlight-calendar/              1.0
  sunlight-chronos/               1.0
  sunlight-emoji-picker/          1.0
  sunlight-files/                 1.0
  sunlight-hangman/               1.0
  sunlight-light-lens/            1.0
  sunlight-reminders/             1.0
  sunlight-wallpaper/             1.0
  sunlight-writer/                1.0
  sunlight-silicon-echoes/        1.0
  sunlight-bench/                 1.0
  sunlight-api-lab/               1.0
  chronos-core/                   1.0
  ChronosDosShell.sunapp/         1.0
  ChronosFileLab.sunapp/          1.0
  ChronosMzLab.sunapp/            1.0
  SunlightMines.sunapp/           1.0
  certificatectl/                 1.0
  golden-fish/                    1.0
  guest/                          1.0
  helios-note/                    1.0
  hello-linux/                    1.0
  rappid-rabbit/                  1.0
  std-proof/                      1.0
  sun-font/                       1.0
  sun-img/                        1.0
  sun-imgc/                       1.0
  structurally recognised apps    1.0
  (everything else)               0.0   (docs, tools, assets, etc.)

Known weighted-v1 limitation: PATCH measures net weighted line growth.  Work
dominated by deletions, or refactoring that keeps approximately the same line
count, may not advance PATCH proportionally.  A future algorithm revision may
address that limitation without changing the canonical version manually.

State compatibility: schema-1 weighted-v1 state is upgraded in place when
saved.  The preceding unversioned raw-line state is migrated by preserving its
version floor and rebasing only the weighted-line baseline.  Unknown algorithms,
future schemas, corrupt JSON, and state newer than canonical repository time are
rejected instead of being silently reset.  --dry-run/--inspect never writes.
"""

import argparse
import datetime
import json
import math
import os
import re
import subprocess
import sys
from collections.abc import Mapping
from pathlib import Path

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

PROJECT_EPOCH = datetime.date(2026, 6, 6)
CYCLE_MONTHS = 30
LINES_PER_PATCH_UNIT = 100
STATE_FILE = ".version_state.json"
ALGORITHM_VERSION = "weighted-v1"
STATE_SCHEMA_VERSION = 2
PLANET_NAMES = (
    "Mercury",
    "Venus",
    "Earth",
    "Mars",
    "Jupiter",
    "Saturn",
    "Uranus",
    "Neptune",
)

# Prerelease schedule: (months_in_cycle, label)
# The first matching threshold wins (list is ordered).
PRERELEASE_SCHEDULE = [
    (3, "alpha.1"),
    (6, "alpha.2"),
    (12, "alpha.3"),
    (15, "beta.1"),
    (18, "beta.2"),
    (21, "rc.1"),
    (30, "rc.2"),
]

# Weight map: ordered list of (dir_prefix, weight).  First match wins.
WEIGHT_MAP = [
    ("kernel/", 4.0),
    ("ipc/", 3.0),
    ("drivers/", 3.0),
    ("compat-linux/", 2.5),
    ("services/", 2.0),
    ("sunlight-fs/", 1.8),
    ("sunlight-fat/", 1.8),
    ("sunlight-block/", 1.8),
    ("sunlight-virtio/", 1.8),
    ("sunlight-net/", 1.8),
    ("sunlight-tls/", 1.8),
    ("sunlight-http/", 1.8),
    ("sunlight-net-utils/", 1.8),
    ("sunlight-fetch/", 1.8),
    ("sunlight-devices/", 1.8),
    ("sunlight-libc/", 1.5),
    ("sunlight-elf/", 1.5),
    ("sunshell/", 1.5),
    ("sunlight-tty/", 1.5),
    ("sunlight-tui/", 1.5),
    ("sunlight-ui/", 1.5),
    ("sunlight-utils/", 1.5),
    ("sunlight-tz/", 1.5),
    ("sunlight-kv/", 1.5),
    ("sunlight-kvctl/", 1.5),
    ("sunlight-locale/", 1.5),
    ("sunlight-telemetry/", 1.5),
    ("sunlight-shell-appstate/", 1.5),
    ("sunlight-top/", 1.5),
    ("sunlight-dialogs/", 1.5),
    ("sunlight-edit/", 1.5),
    ("sunlight-clipman/", 1.5),
    ("cpu-utils/", 1.5),
    ("sunlight-calculator/", 1.0),
    ("sunlight-calendar/", 1.0),
    ("sunlight-chronos/", 1.0),
    ("sunlight-emoji-picker/", 1.0),
    ("sunlight-files/", 1.0),
    ("sunlight-hangman/", 1.0),
    ("sunlight-light-lens/", 1.0),
    ("sunlight-reminders/", 1.0),
    ("sunlight-wallpaper/", 1.0),
    ("sunlight-writer/", 1.0),
    ("sunlight-silicon-echoes/", 1.0),
    ("sunlight-bench/", 1.0),
    ("sunlight-api-lab/", 1.0),
    ("chronos-core/", 1.0),
    ("ChronosDosShell.sunapp/", 1.0),
    ("ChronosFileLab.sunapp/", 1.0),
    ("ChronosMzLab.sunapp/", 1.0),
    ("SunlightMines.sunapp/", 1.0),
    ("certificatectl/", 1.0),
    ("golden-fish/", 1.0),
    ("guest/", 1.0),
    ("helios-note/", 1.0),
    ("hello-linux/", 1.0),
    ("rappid-rabbit/", 1.0),
    ("std-proof/", 1.0),
    ("sun-font/", 1.0),
    ("sun-img/", 1.0),
    ("sun-imgc/", 1.0),
]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

class VersionStateError(RuntimeError):
    """Raised when version state is invalid, stale, or incompatible."""


def _get_weight(rel_path: str, project_root: str | None = None) -> float:
    """Return the weight for a file, based on its directory prefix."""
    for prefix, weight in WEIGHT_MAP:
        if rel_path.startswith(prefix):
            return weight

    parts = Path(rel_path).parts
    if project_root and parts:
        crate_root = Path(project_root) / parts[0]
        has_manifest = (crate_root / "Cargo.toml").is_file()
        has_binary = (
            (crate_root / "src" / "main.rs").is_file()
            or (crate_root / "src" / "bin").is_dir()
        )
        if has_manifest and has_binary:
            return 1.0

    if parts and parts[0].endswith(".sunapp"):
        return 1.0
    return 0.0


def _add_months(date: datetime.date, months: int) -> datetime.date:
    """Add calendar months, clamping the day to the target month's last day."""
    month_index = date.year * 12 + date.month - 1 + months
    year, zero_based_month = divmod(month_index, 12)
    month = zero_based_month + 1
    next_month = datetime.date(year + (month == 12), month % 12 + 1, 1)
    last_day = (next_month - datetime.timedelta(days=1)).day
    return datetime.date(year, month, min(date.day, last_day))


def _full_months_since(start: datetime.date, date: datetime.date) -> int:
    """Return completed calendar months from start to date."""
    if date < start:
        raise ValueError(
            f"canonical date {date.isoformat()} precedes project epoch "
            f"{start.isoformat()}"
        )
    months = (date.year - start.year) * 12 + date.month - start.month
    if date.day < start.day:
        months -= 1
    return months


def _months_since_epoch(date: datetime.date) -> int:
    """Whole calendar months elapsed since the project epoch."""
    return _full_months_since(PROJECT_EPOCH, date)


def _prerelease_label(months_in_cycle: int) -> str:
    """Return the prerelease label for a given position in the cycle."""
    if not 0 <= months_in_cycle < CYCLE_MONTHS:
        raise ValueError(f"months_in_cycle must be in 0..{CYCLE_MONTHS - 1}")
    for threshold, label in PRERELEASE_SCHEDULE:
        if months_in_cycle < threshold:
            return label
    raise AssertionError("prerelease schedule does not cover the full cycle")


def _planetary_boundary(major: int) -> datetime.date:
    """Return the exact stable boundary for a planetary major."""
    if major < 1:
        raise ValueError("planetary majors start at 1")
    return _add_months(PROJECT_EPOCH, major * CYCLE_MONTHS)


def calculate_time_identity(
    canonical_date: datetime.date,
    patch: int,
    *,
    minor_offset_major: int | None = None,
    minor_offset: int = 0,
) -> dict:
    """Calculate the pure time-derived semantic identity."""
    if patch < 0:
        raise ValueError("patch must be non-negative")

    months_total = _months_since_epoch(canonical_date)
    major = months_total // CYCLE_MONTHS
    boundary = major >= 1 and canonical_date == _planetary_boundary(major)
    months_in_cycle = months_total % CYCLE_MONTHS

    if boundary:
        return {
            "version": f"{major}.0.0",
            "major": major,
            "minor": 0,
            "patch": 0,
            "prerelease": None,
            "release_name": (
                PLANET_NAMES[major - 1] if major <= len(PLANET_NAMES) else None
            ),
            "months_total": months_total,
            "months_in_cycle": 0,
            "stable_boundary": True,
        }

    minor = months_in_cycle // 6 + 1
    if minor_offset_major == major:
        minor += minor_offset
    prerelease = _prerelease_label(months_in_cycle)
    return {
        "version": f"{major}.{minor}.{patch}-{prerelease}",
        "major": major,
        "minor": minor,
        "patch": patch,
        "prerelease": prerelease,
        "release_name": None,
        "months_total": months_total,
        "months_in_cycle": months_in_cycle,
        "stable_boundary": False,
    }


def _parse_semver(version: str) -> tuple[tuple[int, int, int], tuple[str, ...] | None]:
    """Parse the subset of SemVer emitted by this tool."""
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?", version)
    if not match:
        raise VersionStateError(f"invalid semantic version in state: {version!r}")
    core = tuple(int(match.group(index)) for index in range(1, 4))
    prerelease = tuple(match.group(4).split(".")) if match.group(4) else None
    return core, prerelease


def compare_semver(left: str, right: str) -> int:
    """Compare two emitted versions using SemVer precedence."""
    left_core, left_pre = _parse_semver(left)
    right_core, right_pre = _parse_semver(right)
    if left_core != right_core:
        return (left_core > right_core) - (left_core < right_core)
    if left_pre is None or right_pre is None:
        if left_pre is right_pre:
            return 0
        return 1 if left_pre is None else -1

    for left_id, right_id in zip(left_pre, right_pre):
        if left_id == right_id:
            continue
        left_numeric = left_id.isdigit()
        right_numeric = right_id.isdigit()
        if left_numeric and right_numeric:
            return (int(left_id) > int(right_id)) - (int(left_id) < int(right_id))
        if left_numeric != right_numeric:
            return -1 if left_numeric else 1
        return (left_id > right_id) - (left_id < right_id)
    return (len(left_pre) > len(right_pre)) - (len(left_pre) < len(right_pre))


def resolve_canonical_datetime(
    project_root: str,
    environ: Mapping[str, str] | None = None,
) -> tuple[datetime.datetime, str]:
    """Resolve canonical repository time from SOURCE_DATE_EPOCH or HEAD."""
    environment = os.environ if environ is None else environ
    source_date_epoch = environment.get("SOURCE_DATE_EPOCH")
    if source_date_epoch is not None:
        try:
            epoch_seconds = int(source_date_epoch)
            if epoch_seconds < 0:
                raise ValueError
        except ValueError as error:
            raise RuntimeError(
                "SOURCE_DATE_EPOCH must be a non-negative integer"
            ) from error
        return (
            datetime.datetime.fromtimestamp(epoch_seconds, datetime.UTC),
            "SOURCE_DATE_EPOCH",
        )

    try:
        result = subprocess.run(
            ["git", "-C", project_root, "log", "-1", "--format=%ct", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        )
        epoch_seconds = int(result.stdout.strip())
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        raise RuntimeError(
            "could not determine canonical version time from HEAD; "
            "set SOURCE_DATE_EPOCH for a reproducible non-Git build"
        ) from error
    return datetime.datetime.fromtimestamp(epoch_seconds, datetime.UTC), "HEAD"


# ---------------------------------------------------------------------------
# Line counting
# ---------------------------------------------------------------------------

def count_weighted_lines(project_root: str) -> tuple[float, dict[str, list[float]]]:
    """Recursively count .rs lines with weights, skipping target/ and hidden dirs.

    Returns (total_weighted_lines, weight_breakdown).
    weight_breakdown maps weight -> [raw_line_counts_for_that_weight].
    """
    total_weighted = 0.0
    breakdown: dict[str, list[float]] = {}  # str key for json compat
    root_path = Path(project_root)

    for rs_file in root_path.rglob("*.rs"):
        parts = rs_file.relative_to(root_path).parts
        if any(part.startswith(".") or part == "target" for part in parts):
            continue

        try:
            with open(rs_file, encoding="utf-8", errors="ignore") as f:
                raw_lines = sum(1 for _ in f)
        except OSError as e:
            print(f"  Warning: Could not read {rs_file}: {e}")
            continue

        rel = str(rs_file.relative_to(root_path))
        weight = _get_weight(rel, project_root)
        key = f"{weight:.1f}"

        if key not in breakdown:
            breakdown[key] = []
        breakdown[key].append(raw_lines)

        total_weighted += raw_lines * weight

    return total_weighted, breakdown


def count_raw_lines(project_root: str) -> int:
    """Recursively count all .rs lines (unweighted), for reporting."""
    root_path = Path(project_root)
    raw = 0
    for rs_file in root_path.rglob("*.rs"):
        parts = rs_file.relative_to(root_path).parts
        if any(part.startswith(".") or part == "target" for part in parts):
            continue
        try:
            with open(rs_file, encoding="utf-8", errors="ignore") as f:
                raw += sum(1 for _ in f)
        except OSError:
            pass
    return raw


# ---------------------------------------------------------------------------
# State management
# ---------------------------------------------------------------------------

def _fresh_state() -> dict:
    """Return a new weighted-v1 state without selecting a baseline."""
    return {
        "state_schema": STATE_SCHEMA_VERSION,
        "version_algorithm": ALGORITHM_VERSION,
        "last_known_weighted_lines": None,
        "accumulated_patch": 0,
        "last_version": None,
        "last_total_raw_lines": None,
        "last_total_weighted_lines": None,
        "last_canonical_epoch": None,
        "minor_offset_major": None,
        "minor_offset": 0,
        "history": [],
    }


def _require_non_negative_int(state: dict, field: str, default: int = 0) -> int:
    value = state.get(field, default)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        raise VersionStateError(
            f"{STATE_FILE} field {field!r} must be a non-negative integer"
        )
    return value


def load_state(project_root: str) -> dict:
    """Load state, recognising only weighted-v1 and the explicit legacy format."""
    state_path = os.path.join(project_root, STATE_FILE)

    if not os.path.exists(state_path):
        print("  Creating fresh state...")
        state = _fresh_state()
        state["_fresh_state"] = True
        return state

    try:
        with open(state_path, encoding="utf-8") as file:
            state = json.load(file)
    except (json.JSONDecodeError, OSError) as error:
        raise VersionStateError(
            f"could not read valid JSON from {STATE_FILE}: {error}"
        ) from error

    if not isinstance(state, dict):
        raise VersionStateError(f"{STATE_FILE} must contain a JSON object")

    algorithm = state.get("version_algorithm")
    if algorithm is None and (
        "last_known_lines" in state or "last_total_lines" in state
    ):
        state["_migration_source"] = "legacy-raw-v0"
        print(f"  Found legacy raw-line state in {STATE_FILE}; migration required")
        return state

    if algorithm != ALGORITHM_VERSION:
        raise VersionStateError(
            f"incompatible version state algorithm {algorithm!r}; "
            f"expected {ALGORITHM_VERSION!r}. An explicit migration is required."
        )
    if "last_known_weighted_lines" not in state:
        raise VersionStateError(
            f"stale {ALGORITHM_VERSION} state is missing "
            "'last_known_weighted_lines'"
        )
    state_schema = state.get("state_schema", 1)
    if (
        not isinstance(state_schema, int)
        or isinstance(state_schema, bool)
        or state_schema < 1
        or state_schema > STATE_SCHEMA_VERSION
    ):
        raise VersionStateError(
            f"incompatible {STATE_FILE} schema {state_schema!r}; "
            f"supported schemas are 1 through {STATE_SCHEMA_VERSION}"
        )

    _require_non_negative_int(state, "accumulated_patch")
    if state.get("last_version") is not None:
        _parse_semver(state["last_version"])
    if not isinstance(state.get("history", []), list):
        raise VersionStateError(f"{STATE_FILE} field 'history' must be a list")

    print(f"  Loaded existing state ({ALGORITHM_VERSION}) from {STATE_FILE}")
    return state


def _normalise_state(
    state: dict,
    current_weighted: float,
    canonical_date: datetime.date,
) -> tuple[dict, str | None]:
    """Migrate supported state deterministically and validate continuity."""
    migration_source = state.get("_migration_source")
    if migration_source == "legacy-raw-v0":
        accumulated_patch = _require_non_negative_int(state, "accumulated_patch")
        last_version = state.get("last_version")
        if last_version is not None:
            previous_core, _ = _parse_semver(last_version)
        else:
            previous_core = None

        natural_identity = calculate_time_identity(canonical_date, accumulated_patch)
        minor_offset_major = None
        minor_offset = 0
        if (
            previous_core is not None
            and not natural_identity["stable_boundary"]
            and previous_core[0] == natural_identity["major"]
            and previous_core[1] > natural_identity["minor"]
        ):
            minor_offset_major = natural_identity["major"]
            minor_offset = previous_core[1] - natural_identity["minor"]

        migrated_identity = calculate_time_identity(
            canonical_date,
            accumulated_patch,
            minor_offset_major=minor_offset_major,
            minor_offset=minor_offset,
        )
        if last_version is not None:
            if previous_core[0] > migrated_identity["major"]:
                raise VersionStateError(
                    "legacy state last_version is ahead of canonical repository time"
                )
            if compare_semver(migrated_identity["version"], last_version) < 0:
                if migrated_identity["stable_boundary"]:
                    raise VersionStateError(
                        "legacy state cannot be migrated without crossing a "
                        "planetary stable boundary"
                    )
                if (
                    previous_core[0] == migrated_identity["major"]
                    and previous_core[1] == migrated_identity["minor"]
                ):
                    accumulated_patch = max(
                        accumulated_patch,
                        previous_core[2],
                    )
                migrated_identity = calculate_time_identity(
                    canonical_date,
                    accumulated_patch,
                    minor_offset_major=minor_offset_major,
                    minor_offset=minor_offset,
                )
                if compare_semver(migrated_identity["version"], last_version) < 0:
                    accumulated_patch += 1
                migrated_identity = calculate_time_identity(
                    canonical_date,
                    accumulated_patch,
                    minor_offset_major=minor_offset_major,
                    minor_offset=minor_offset,
                )
                if compare_semver(migrated_identity["version"], last_version) < 0:
                    raise VersionStateError(
                        "legacy state cannot be migrated monotonically"
                    )

        normalised = _fresh_state()
        normalised.update(
            {
                "last_known_weighted_lines": current_weighted,
                "accumulated_patch": accumulated_patch,
                "last_version": last_version,
                "last_total_raw_lines": state.get("last_total_lines"),
                "last_total_weighted_lines": current_weighted,
                "minor_offset_major": minor_offset_major,
                "minor_offset": minor_offset,
                "history": state.get("history", [])
                if isinstance(state.get("history", []), list)
                else [],
            }
        )
        diagnostic = (
            "migrated legacy raw-line baseline to weighted-v1 at the current "
            f"weighted total; preserved monotonic version floor at patch "
            f"{accumulated_patch}"
        )
        print(f"  State migration: {diagnostic}")
        return normalised, diagnostic

    normalised = _fresh_state()
    normalised.update(state)
    normalised.pop("_migration_source", None)

    baseline = normalised.get("last_known_weighted_lines")
    if normalised.pop("_fresh_state", False):
        normalised["last_known_weighted_lines"] = current_weighted
        normalised["last_total_weighted_lines"] = current_weighted
        return normalised, None
    if not isinstance(baseline, (int, float)) or isinstance(baseline, bool):
        raise VersionStateError(
            f"{STATE_FILE} field 'last_known_weighted_lines' must be numeric"
        )
    if not math.isfinite(float(baseline)) or baseline < 0:
        raise VersionStateError(
            f"{STATE_FILE} field 'last_known_weighted_lines' must be finite "
            "and non-negative"
        )

    minor_offset = _require_non_negative_int(normalised, "minor_offset")
    minor_offset_major = normalised.get("minor_offset_major")
    if minor_offset_major is not None and (
        not isinstance(minor_offset_major, int)
        or isinstance(minor_offset_major, bool)
        or minor_offset_major < 0
    ):
        raise VersionStateError(
            f"{STATE_FILE} field 'minor_offset_major' must be null or "
            "a non-negative integer"
        )
    if minor_offset and minor_offset_major is None:
        raise VersionStateError(
            f"{STATE_FILE} has a minor offset without its generation major"
        )

    return normalised, None


def save_state(project_root: str, state: dict) -> None:
    """Save version state to .version_state.json."""
    state_path = os.path.join(project_root, STATE_FILE)
    with open(state_path, "w", encoding="utf-8") as f:
        json.dump(state, f, indent=2)
    print(f"  State saved to {STATE_FILE}")


# ---------------------------------------------------------------------------
# Version calculation
# ---------------------------------------------------------------------------

def calculate_version(
    project_root: str,
    *,
    canonical_datetime: datetime.datetime | None = None,
    canonical_source: str | None = None,
) -> tuple[str, dict, dict]:
    """Calculate version from canonical repository time and weighted growth.

    Returns (version_string, new_state, details).
    """
    if canonical_datetime is None:
        canonical_datetime, resolved_source = resolve_canonical_datetime(project_root)
        canonical_source = resolved_source
    elif canonical_datetime.tzinfo is None:
        canonical_datetime = canonical_datetime.replace(tzinfo=datetime.UTC)
    canonical_datetime = canonical_datetime.astimezone(datetime.UTC)
    canonical_source = canonical_source or "provided"
    canonical_date = canonical_datetime.date()
    canonical_epoch = int(canonical_datetime.timestamp())

    current_weighted, breakdown = count_weighted_lines(project_root)
    current_raw = count_raw_lines(project_root)
    loaded_state = load_state(project_root)
    state, migration = _normalise_state(
        loaded_state, current_weighted, canonical_date
    )

    previous_epoch = state.get("last_canonical_epoch")
    if previous_epoch is not None:
        if (
            not isinstance(previous_epoch, int)
            or isinstance(previous_epoch, bool)
            or previous_epoch < 0
        ):
            raise VersionStateError(
                f"{STATE_FILE} field 'last_canonical_epoch' must be null or "
                "a non-negative integer"
            )
        if previous_epoch > canonical_epoch:
            raise VersionStateError(
                f"stale {STATE_FILE}: recorded canonical time {previous_epoch} "
                f"is newer than current canonical time {canonical_epoch}"
            )

    baseline_w = float(state["last_known_weighted_lines"])

    acc_patch = state.get("accumulated_patch", 0)
    _ltw = state.get("last_total_weighted_lines")
    last_total_weighted = _ltw if _ltw is not None else current_weighted

    new_weighted = current_weighted - baseline_w

    if new_weighted > 0:
        patch_increment = math.ceil(new_weighted / LINES_PER_PATCH_UNIT)
    else:
        patch_increment = 0
        real_delta = current_weighted - last_total_weighted
        if real_delta < 0:
            print(f"  Warning: Weighted line count decreased by {abs(real_delta):.1f} "
                  "since last recorded total (refactoring?)")
            print("  Patch version will not decrease")

    new_patch = acc_patch + patch_increment
    identity = calculate_time_identity(
        canonical_date,
        new_patch,
        minor_offset_major=state.get("minor_offset_major"),
        minor_offset=state.get("minor_offset", 0),
    )
    version_string = identity["version"]
    if (
        state.get("minor_offset_major") is not None
        and identity["major"] > state["minor_offset_major"]
    ):
        state["minor_offset_major"] = None
        state["minor_offset"] = 0

    previous_version = state.get("last_version")
    if previous_version is not None and compare_semver(
        version_string, previous_version
    ) < 0:
        raise VersionStateError(
            f"calculated version {version_string} would precede recorded "
            f"version {previous_version}; state or canonical time is stale"
        )

    consumed_weighted = patch_increment * LINES_PER_PATCH_UNIT
    updated_baseline = (
        baseline_w + consumed_weighted if new_weighted > 0 else baseline_w
    )

    history_entry = {
        "timestamp": canonical_datetime.isoformat(),
        "canonical_source": canonical_source,
        "version": version_string,
        "major": identity["major"],
        "minor": identity["minor"],
        "patch": identity["patch"],
        "prerelease": identity["prerelease"],
        "release_name": identity["release_name"],
        "months_total": identity["months_total"],
        "months_in_cycle": identity["months_in_cycle"],
        "total_raw_lines": current_raw,
        "total_weighted_lines": current_weighted,
        "new_weighted_lines": new_weighted,
        "patch_increment": patch_increment,
    }

    history = state.get("history", [])
    if not isinstance(history, list):
        raise VersionStateError(f"{STATE_FILE} field 'history' must be a list")
    if not history or history[-1] != history_entry:
        history.append(history_entry)

    new_state = {
        "state_schema": STATE_SCHEMA_VERSION,
        "version_algorithm": ALGORITHM_VERSION,
        "last_known_weighted_lines": updated_baseline,
        "accumulated_patch": new_patch,
        "last_version": version_string,
        "last_total_raw_lines": current_raw,
        "last_total_weighted_lines": current_weighted,
        "last_canonical_epoch": canonical_epoch,
        "minor_offset_major": state.get("minor_offset_major"),
        "minor_offset": state.get("minor_offset", 0),
        "history": history[-50:],
    }

    details = {
        **identity,
        "canonical_datetime": canonical_datetime,
        "canonical_source": canonical_source,
        "current_raw_lines": current_raw,
        "current_weighted_lines": current_weighted,
        "last_known_weighted_lines": baseline_w,
        "new_weighted_lines": new_weighted,
        "patch_increment": patch_increment,
        "breakdown": breakdown,
        "state_migration": migration,
    }

    return version_string, new_state, details


# ---------------------------------------------------------------------------
# Cargo.toml update
# ---------------------------------------------------------------------------

def find_cargo_tomls(project_root: str) -> list[str]:
    """Find all Cargo.toml files (excluding target/ and hidden dirs)."""
    cargo_files = []
    root_path = Path(project_root)
    for cargo_file in root_path.rglob("Cargo.toml"):
        parts = cargo_file.relative_to(root_path).parts
        if any(part.startswith(".") or part == "target" for part in parts):
            continue
        cargo_files.append(str(cargo_file))
    return cargo_files


def update_cargo_toml(file_path: str, new_version: str) -> str:
    """Update version fields. Returns status string."""
    try:
        with open(file_path, "r", encoding="utf-8") as f:
            lines = f.readlines()
    except Exception as e:
        print(f"  ⚠ Could not read {file_path}: {e}")
        return "error"

    current_section = None
    changed = False
    found_editable_version = False
    found_workspace_inherited = False
    new_lines = []

    for line in lines:
        stripped = line.strip()

        if stripped.startswith("[") and stripped.endswith("]"):
            current_section = stripped

        if current_section == "[package]":
            if re.match(r'^\s*version\.workspace\s*=\s*true\s*(#.*)?$', line):
                found_workspace_inherited = True

            m = re.match(r'^(\s*version\s*=\s*)"([^"]*)"(\s*(#.*)?)?$', line)
            if m:
                found_editable_version = True
                old_version = m.group(2)
                if old_version != new_version:
                    suffix = m.group(3) or ""
                    nl = "\n" if line.endswith("\n") else ""
                    line = f'{m.group(1)}"{new_version}"{suffix}{nl}'
                    changed = True

        elif current_section == "[workspace.package]":
            m = re.match(r'^(\s*version\s*=\s*)"([^"]*)"(\s*(#.*)?)?$', line)
            if m:
                found_editable_version = True
                old_version = m.group(2)
                if old_version != new_version:
                    suffix = m.group(3) or ""
                    nl = "\n" if line.endswith("\n") else ""
                    line = f'{m.group(1)}"{new_version}"{suffix}{nl}'
                    changed = True

        new_lines.append(line)

    if changed:
        try:
            with open(file_path, "w", encoding="utf-8") as f:
                f.writelines(new_lines)
            return "updated"
        except Exception as e:
            print(f"  ⚠ Could not write {file_path}: {e}")
            return "error"

    if found_editable_version:
        return "already-current"
    if found_workspace_inherited:
        return "workspace-inherited"
    return "no-version-field"


# ---------------------------------------------------------------------------
# Display
# ---------------------------------------------------------------------------

def print_banner() -> None:
    print()
    print("=" * 68)
    print("  SunlightOS Auto Version Manager")
    print()
    print("  MAJOR = exact 30-month planetary generation boundary")
    print("  MINOR = one-based six-month index within the generation")
    print("  PATCH = accumulated(ceil(weighted new lines / 100))")
    print()
    print("  Cycle stages: alpha.1 → alpha.2 → alpha.3 → beta.1 →")
    print("                beta.2 → rc.1 → rc.2 → exact stable boundary")
    print()
    print("  Weights: kernel 4.0 | ABI/drivers 3.0 | compat 2.5 |")
    print("           services 2.0 | infra 1.8 | core 1.5 | apps 1.0")
    print("=" * 68)
    print()


def print_report(
    version: str,
    details: dict,
    cargo_files_updated: list[str],
    already_current: list[str] | None = None,
    workspace_inherited: list[str] | None = None,
    no_version_field: list[str] | None = None,
    errors: list[str] | None = None,
) -> None:
    project_date = datetime.date.today()
    project_day = max((project_date - PROJECT_EPOCH).days + 1, 0)
    already_current = already_current or []
    workspace_inherited = workspace_inherited or []
    no_version_field = no_version_field or []
    errors = errors or []

    pr_label = details["prerelease"] or "(none — stable boundary)"
    release_name = details.get("release_name")
    display_version = version
    if release_name:
        display_version = f'{version} "{release_name}"'

    print("-" * 68)
    print("  Version Calculation Report")
    print("-" * 68)
    print(
        f"  Canonical timestamp:   "
        f"{details['canonical_datetime'].isoformat()} ({details['canonical_source']})"
    )
    print(f"  Project Day:           {project_day} ({project_date.isoformat()})")
    print(f"  Months since epoch:    {details['months_total']}")
    print(f"  Months in cycle:       {details['months_in_cycle']}")
    print(f"  MAJOR (generation):    {details['major']}")
    print(f"  MINOR (cycle half):    {details['minor']}")
    print(f"  PATCH (weighted):      {details['patch']}")
    print(f"  Prerelease label:      {pr_label}")
    print(f"  ---")
    print(f"  Raw .rs lines:         {details['current_raw_lines']}")
    print(f"  Weighted .rs lines:    {details['current_weighted_lines']:.1f}")
    print(f"  Previous weighted:     {details['last_known_weighted_lines']:.1f}")
    print(f"  New weighted lines:    {details['new_weighted_lines']:.1f}")
    print(f"  Patch increment:       +{details['patch_increment']}")
    print(f"  NEW VERSION:           {display_version}")
    if details.get("state_migration"):
        print(f"  State migration:       {details['state_migration']}")

    # Line breakdown by weight
    bd = details.get("breakdown", {})
    if bd:
        print(f"  ---")
        print(f"  Lines by weight tier:")
        for weight_key in sorted(bd.keys(), key=lambda k: float(k), reverse=True):
            counts = bd[weight_key]
            raw_sum = int(sum(counts))
            weighted_sum = raw_sum * float(weight_key)
            print(f"    weight {float(weight_key):.1f}: "
                  f"{raw_sum:>7,} raw -> {weighted_sum:>9,.1f} weighted "
                  f"({len(counts)} files)")

    total = (len(cargo_files_updated) + len(already_current) +
             len(workspace_inherited) + len(no_version_field) + len(errors))

    if cargo_files_updated:
        print(f"  ---")
        print(f"  Updated Cargo.toml ({len(cargo_files_updated)}):")
        for f in cargo_files_updated:
            print(f"    - {os.path.relpath(f)}")

    if already_current:
        print(f"  Already current ({len(already_current)})")
    if workspace_inherited:
        print(f"  Uses version.workspace = true ({len(workspace_inherited)})")
    if no_version_field:
        print(f"  No editable version field ({len(no_version_field)})")
    if errors:
        print(f"  Errors ({len(errors)})")
        for f in errors:
            print(f"    - {os.path.relpath(f)}")

    if not cargo_files_updated and not errors:
        if total > 0:
            print(f"  All {total} Cargo.toml files processed (no changes required).")

    print("-" * 68)
    print()


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def _parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Calculate the SunlightOS version")
    parser.add_argument(
        "project_root",
        nargs="?",
        default=os.getcwd(),
        help="repository root (default: current directory)",
    )
    parser.add_argument(
        "--dry-run",
        "--inspect",
        dest="dry_run",
        action="store_true",
        help="calculate and report without modifying Cargo.toml or version state",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> str:
    args = _parse_args(argv)
    project_root = os.path.abspath(args.project_root)

    print_banner()
    print(f"  Project root: {project_root}")
    if args.dry_run:
        print("  Mode: dry-run (no files will be modified)")
    print()

    print("  Counting weighted .rs file lines...")
    version, new_state, details = calculate_version(project_root)
    print(f"  Calculated version: {version}")
    print()

    updated_files = []
    already_current = []
    workspace_inherited = []
    no_version_field = []
    errors = []

    if args.dry_run:
        print("  Dry-run complete; Cargo.toml files and state were not modified.")
        print()
    else:
        print("  Searching for Cargo.toml files...")
        cargo_files = find_cargo_tomls(project_root)
        print(f"  Found {len(cargo_files)} Cargo.toml file(s)")
        print()

        for cargo_file in cargo_files:
            rel_path = os.path.relpath(cargo_file, project_root)
            status = update_cargo_toml(cargo_file, version)
            if status == "updated":
                print(f"  Processing {rel_path}... updated")
                updated_files.append(cargo_file)
            elif status == "already-current":
                print(f"  Processing {rel_path}... already current")
                already_current.append(cargo_file)
            elif status == "workspace-inherited":
                print(f"  Processing {rel_path}... uses version.workspace = true")
                workspace_inherited.append(cargo_file)
            elif status == "no-version-field":
                print(f"  Processing {rel_path}... no editable version field")
                no_version_field.append(cargo_file)
            else:
                print(f"  Processing {rel_path}... error")
                errors.append(cargo_file)

        print()
        save_state(project_root, new_state)
        print()

    print_report(version, details, updated_files, already_current,
                 workspace_inherited, no_version_field, errors)

    return version


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, ValueError) as error:
        print(f"  Error: {error}", file=sys.stderr)
        raise SystemExit(2) from error
