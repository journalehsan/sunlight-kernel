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
        "last_total_lines": BASE_LINES,
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
    last_total_lines = state.get("last_total_lines", last_known_lines)

    new_lines = current_lines - last_known_lines

    if new_lines > 0:
        # 1-100 new lines -> +1, 101-200 -> +2, etc. (any positive change under 100 now bumps patch)
        patch_increment = (new_lines + (LINES_PER_PATCH_UNIT - 1)) // LINES_PER_PATCH_UNIT
    else:
        patch_increment = 0
        # Only warn about real shrinkage vs the last *observed* total (not our internal accounting baseline)
        real_delta = current_lines - last_total_lines
        if real_delta < 0:
            print(f"  Warning: Line count decreased by {abs(real_delta)} lines since last recorded total (refactoring?)")
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
        "last_total_lines": current_lines,   # actual observed total (for better decrease detection)
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


def update_cargo_toml(file_path: str, new_version: str) -> str:
    """
    Update version in:
    - [package] -> version = "..."
    - [workspace.package] -> version = "..."

    Returns one of:
    - "updated"
    - "already-current"
    - "workspace-inherited"
    - "no-version-field"
    - "error"
    """
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

        # section tracking
        if stripped.startswith("[") and stripped.endswith("]"):
            current_section = stripped

        # detect inherited workspace version in [package]
        if current_section == "[package]":
            if re.match(r'^\s*version\.workspace\s*=\s*true\s*(#.*)?$', line):
                found_workspace_inherited = True

            m = re.match(r'^(\s*version\s*=\s*)"([^"]*)"(\s*(#.*)?)?$', line)
            if m:
                found_editable_version = True
                old_version = m.group(2)
                if old_version != new_version:
                    suffix = m.group(3) or ""
                    newline = "\n" if line.endswith("\n") else ""
                    line = f'{m.group(1)}"{new_version}"{suffix}{newline}'
                    changed = True

        # support workspace.package version
        elif current_section == "[workspace.package]":
            m = re.match(r'^(\s*version\s*=\s*)"([^"]*)"(\s*(#.*)?)?$', line)
            if m:
                found_editable_version = True
                old_version = m.group(2)
                if old_version != new_version:
                    suffix = m.group(3) or ""
                    newline = "\n" if line.endswith("\n") else ""
                    line = f'{m.group(1)}"{new_version}"{suffix}{newline}'
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


def print_report(
    version: str,
    details: dict,
    cargo_files_updated: list[str],
    already_current: list[str] | None = None,
    workspace_inherited: list[str] | None = None,
    no_version_field: list[str] | None = None,
    errors: list[str] | None = None,
) -> None:
    now = datetime.datetime.now()
    already_current = already_current or []
    workspace_inherited = workspace_inherited or []
    no_version_field = no_version_field or []
    errors = errors or []

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
    rem = details["remainder_lines"]
    if rem < 0:
        print(f"  Remainder lines:  {rem} (patch credit issued early; next bump after {-rem} more lines)")
    else:
        print(f"  Remainder lines:  {rem} (carried to next build)")
    print(f"  NEW VERSION:      {version}")

    total = len(cargo_files_updated) + len(already_current) + len(workspace_inherited) + len(no_version_field) + len(errors)

    if cargo_files_updated:
        print(f"  Updated ({len(cargo_files_updated)}):")
        for cargo_file in cargo_files_updated:
            print(f"    - {os.path.relpath(cargo_file)}")

    if already_current:
        print(f"  Already current ({len(already_current)}):")
        for cargo_file in already_current:
            print(f"    - {os.path.relpath(cargo_file)}")

    if workspace_inherited:
        print(f"  Uses version.workspace = true ({len(workspace_inherited)}):")
        for cargo_file in workspace_inherited:
            print(f"    - {os.path.relpath(cargo_file)}")

    if no_version_field:
        print(f"  No editable version field ({len(no_version_field)}):")
        for cargo_file in no_version_field:
            print(f"    - {os.path.relpath(cargo_file)}")

    if errors:
        print(f"  Errors ({len(errors)}):")
        for cargo_file in errors:
            print(f"    - {os.path.relpath(cargo_file)}")

    if not cargo_files_updated and not errors:
        if total > 0:
            print(f"  All {total} Cargo.toml files processed (no changes required this run).")
        else:
            print("  No Cargo.toml files found or needed updating.")

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

    # Detailed calculation output (as recommended)
    print(f"  Calculated version: {version}")
    print(f"  Current lines:      {details['current_lines']}")
    print(f"  Last known lines:   {details['last_known_lines']}")
    print(f"  New lines:          {details['new_lines']}")
    print(f"  Patch increment:    +{details['patch_increment']}")
    print()

    print("  Searching for Cargo.toml files...")
    cargo_files = find_cargo_tomls(project_root)
    print(f"  Found {len(cargo_files)} Cargo.toml file(s)")
    print()

    # Track all outcomes for better reporting
    updated_files = []
    already_current = []
    workspace_inherited = []
    no_version_field = []
    errors = []

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
    print_report(version, details, updated_files, already_current, workspace_inherited, no_version_field, errors)

    return version


if __name__ == "__main__":
    main()
