#!/usr/bin/env python3
"""
Auto Version Manager for Rust Projects
=======================================
Version format: MAJOR.MINOR.PATCH

- MAJOR: (current_year - base_year) -> 2026=0, 2027=1, 2028=2, ...
- MINOR: 1 if month <= 6, 2 if month > 6
- PATCH: base_patch + ceil(new_lines_since_last_calc / 100)
         (1-100 new lines since last run -> +1 patch, 101-200 -> +2, etc.)
         where new_lines = current_total_lines - saved_total_lines

Base version: 0.1.0
Base year:    2026
Base lines:   33535
"""

import datetime
import json
import os
import re
import sys
from pathlib import Path

BASE_YEAR = 2026
BASE_VERSION_MAJOR = 0
BASE_VERSION_MINOR = 1
BASE_VERSION_PATCH = 0
BASE_LINES = 33535
LINES_PER_PATCH_UNIT = 100
STATE_FILE = ".version_state.json"


def count_rs_lines(project_root: str) -> int:
    """Recursively count all lines in .rs files, skipping target/ and hidden dirs."""
    total_lines = 0
    root_path = Path(project_root)

    for rs_file in root_path.rglob("*.rs"):
        parts = rs_file.relative_to(root_path).parts
        if any(part.startswith(".") or part == "target" for part in parts):
            continue

        try:
            with open(rs_file, encoding="utf-8", errors="ignore") as f:
                total_lines += sum(1 for _ in f)
        except OSError as e:
            print(f"  Warning: Could not read {rs_file}: {e}")

    return total_lines


def load_state(project_root: str) -> dict:
    """Load saved version state, or return defaults if missing."""
    state_path = os.path.join(project_root, STATE_FILE)

    if os.path.exists(state_path):
        try:
            with open(state_path, encoding="utf-8") as f:
                state = json.load(f)
                print(f"  Loaded existing state from {STATE_FILE}")
                return state
        except (json.JSONDecodeError, OSError) as e:
            print(f"  Warning: Could not load state file: {e}")
            print("  Creating fresh state...")

    return {
        "last_known_lines": BASE_LINES,
        "accumulated_patch": BASE_VERSION_PATCH,
        "last_version": f"{BASE_VERSION_MAJOR}.{BASE_VERSION_MINOR}.{BASE_VERSION_PATCH}",
        "history": [],
    }


def save_state(project_root: str, state: dict) -> None:
    """Save version state to .version_state.json."""
    state_path = os.path.join(project_root, STATE_FILE)

    with open(state_path, "w", encoding="utf-8") as f:
        json.dump(state, f, indent=2)

    print(f"  State saved to {STATE_FILE}")


def calculate_version(project_root: str) -> tuple[str, dict, dict]:
    """Calculate version from date and line growth. Returns (version, state, details)."""
    now = datetime.datetime.now()

    major = BASE_VERSION_MAJOR + (now.year - BASE_YEAR)
    if major < 0:
        print(f"  Warning: Current year ({now.year}) is before base year ({BASE_YEAR})")
        print("  Clamping MAJOR to 0")
        major = 0

    minor = 1 if now.month <= 6 else 2

    state = load_state(project_root)
    current_lines = count_rs_lines(project_root)
    last_known_lines = state.get("last_known_lines", BASE_LINES)
    accumulated_patch = state.get("accumulated_patch", BASE_VERSION_PATCH)

    new_lines = current_lines - last_known_lines

    if new_lines > 0:
        patch_increment = new_lines // LINES_PER_PATCH_UNIT
    else:
        patch_increment = 0
        if new_lines < 0:
            print(f"  Warning: Line count decreased by {abs(new_lines)} lines (refactoring?)")
            print("  Patch version will not decrease")

    new_patch = accumulated_patch + patch_increment
    version_string = f"{major}.{minor}.{new_patch}"

    consumed_lines = patch_increment * LINES_PER_PATCH_UNIT
    updated_lines = last_known_lines + consumed_lines if new_lines > 0 else last_known_lines

    history_entry = {
        "timestamp": now.isoformat(),
        "version": version_string,
        "total_lines": current_lines,
        "new_lines": new_lines,
        "patch_increment": patch_increment,
    }

    history = state.get("history", [])
    if not isinstance(history, list):
        history = []
    history.append(history_entry)

    new_state = {
        "last_known_lines": updated_lines,
        "accumulated_patch": new_patch,
        "last_version": version_string,
        "history": history[-50:],
    }

    details = {
        "major": major,
        "minor": minor,
        "patch": new_patch,
        "current_lines": current_lines,
        "last_known_lines": last_known_lines,
        "new_lines": new_lines,
        "patch_increment": patch_increment,
        "consumed_lines": consumed_lines,
        "remainder_lines": new_lines - consumed_lines if new_lines > 0 else 0,
    }

    return version_string, new_state, details


def find_cargo_tomls(project_root: str) -> list[str]:
    """Find all Cargo.toml files in the project (excluding target/)."""
    cargo_files = []
    root_path = Path(project_root)

    for cargo_file in root_path.rglob("Cargo.toml"):
        parts = cargo_file.relative_to(root_path).parts
        if any(part.startswith(".") or part == "target" for part in parts):
            continue
        cargo_files.append(str(cargo_file))

    return cargo_files


def update_cargo_toml(file_path: str, new_version: str) -> bool:
    """Update version in [package] section only. Returns True if modified."""
    try:
        with open(file_path, encoding="utf-8") as f:
            content = f.read()
    except OSError as e:
        print(f"  Warning: Could not read {file_path}: {e}")
        return False

    original_content = content
    lines = content.split("\n")
    new_lines = []
    in_package_section = False
    version_updated = False

    for line in lines:
        stripped = line.strip()

        if stripped.startswith("["):
            in_package_section = stripped == "[package]"

        if in_package_section and not version_updated:
            version_match = re.match(r'^(\s*version\s*=\s*)"[^"]*"(.*)$', line)
            if version_match:
                prefix = version_match.group(1)
                suffix = version_match.group(2)
                line = f'{prefix}"{new_version}"{suffix}'
                version_updated = True

        new_lines.append(line)

    new_content = "\n".join(new_lines)

    if new_content != original_content:
        try:
            with open(file_path, "w", encoding="utf-8") as f:
                f.write(new_content)
            return True
        except OSError as e:
            print(f"  Warning: Could not write {file_path}: {e}")
            return False

    return False


def print_banner() -> None:
    print()
    print("=" * 62)
    print("  Rust Auto Version Manager")
    print()
    print("  Version Schema:")
    print("    MAJOR = current_year - 2026")
    print("    MINOR = 1 (Jan-Jun) | 2 (Jul-Dec)")
    print("    PATCH = accumulated(ceil(new .rs lines / 100))  # 1-100 lines gives +1")
    print("=" * 62)
    print()


def print_report(version: str, details: dict, cargo_files_updated: list[str]) -> None:
    now = datetime.datetime.now()

    print("-" * 62)
    print("  Version Calculation Report")
    print("-" * 62)
    print(f"  Timestamp:        {now.strftime('%Y-%m-%d %H:%M:%S')}")
    print(f"  Current Year:     {now.year}")
    print(f"  Current Month:    {now.month}")
    print(f"  MAJOR version:    {details['major']} (year {now.year} - base {BASE_YEAR})")
    print(
        f"  MINOR version:    {details['minor']} "
        f"(month {now.month} {'<= 6 -> 1' if details['minor'] == 1 else '> 6 -> 2'})"
    )
    print(f"  PATCH version:    {details['patch']}")
    print(f"  Previous lines:   {details['last_known_lines']}")
    print(f"  Current lines:    {details['current_lines']}")
    print(f"  New lines:        {details['new_lines']}")
    print(f"  Patch increment:  +{details['patch_increment']}")
    print(f"  Remainder lines:  {details['remainder_lines']} (carried to next build)")
    print(f"  NEW VERSION:      {version}")

    if cargo_files_updated:
        print("  Updated Cargo.toml files:")
        for cargo_file in cargo_files_updated:
            print(f"    - {os.path.relpath(cargo_file)}")
    else:
        print("  No Cargo.toml files needed updating")

    print("-" * 62)
    print()


def main() -> str:
    if len(sys.argv) > 1:
        project_root = sys.argv[1]
    else:
        project_root = os.getcwd()

    project_root = os.path.abspath(project_root)

    print_banner()
    print(f"  Project root: {project_root}")
    print()

    print("  Counting .rs file lines...")
    version, new_state, details = calculate_version(project_root)
    print(f"  Total .rs lines: {details['current_lines']}")
    print(f"  Calculated version: {version}")
    print()

    print("  Searching for Cargo.toml files...")
    cargo_files = find_cargo_tomls(project_root)
    print(f"  Found {len(cargo_files)} Cargo.toml file(s)")

    updated_files = []
    for cargo_file in cargo_files:
        rel_path = os.path.relpath(cargo_file, project_root)
        print(f"  Processing {rel_path}...", end=" ")
        if update_cargo_toml(cargo_file, version):
            print("updated")
            updated_files.append(cargo_file)
        else:
            print("unchanged")

    print()
    save_state(project_root, new_state)
    print()
    print_report(version, details, updated_files)

    return version


if __name__ == "__main__":
    main()
