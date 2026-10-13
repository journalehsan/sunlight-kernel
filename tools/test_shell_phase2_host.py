#!/usr/bin/env python3
"""Entry-point and supported-subset differential checks; never used as native proof."""
import os
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
REFERENCE = os.environ.get("SSHL_REFERENCE_SHELL", "dash")
SHELL = ROOT / "target/x86_64-unknown-linux-gnu/debug/sshl"
CASES = [
    ("if false; then echo bad; elif true; then if ! false; then echo yes; fi; fi", []),
    ("for x in one two; do for y in a b; do echo $x:$y; done; done", []),
    ("while true; do for x in a b; do echo $x; break 2; done; done", []),
    ('ready=no; until test "$ready" = yes; do ready=yes; done; echo "$ready"', []),
    ('greet() { echo "$0:$#:$1"; return 7; }; greet Sunlight; echo $?; echo "$1"', ["named", "original"]),
    ('for x in "$@"; do echo "<$x>"; done; for x in $@; do echo "[$x]"; done', ["named", "a b", "", "c"]),
    ('set --; for x in "$@"; do echo bad; done; echo "$#"', []),
    ('set -- one two; shift; echo "$#:$1:$*"', []),
    ('IFS=:; V=a::b; for x in $V; do echo "<$x>"; done', []),
    ('value=parent; (value=child; cd /tmp; echo "$value"); echo "$value"; pwd', []),
    ("false && echo bad || echo good; true || echo bad; ! true; echo $?", []),
    ("case abc in 'a*') echo bad;; a?c|def) echo yes;; *) echo bad;; esac", []),
    ("case '*abcx' in *x) echo matched;; *) echo bad;; esac", []),
    ("f() { B=changed; }; A=original; A=temporary f; echo $A:$B", []),
    ("cd /no-such-sshl-directory || echo recovered; echo after", []),
    ("test bad -eq 2; echo status:$?", []),
    ("echo first | cat | cat; false | true; echo $?; exit 23", []),
    ("echo before; exit 9; echo bad", []),
    ("if true; then", []),
    ("export CHILD=visible; /usr/bin/printenv CHILD; readonly LOCK=ok; echo $LOCK", []),
    ('V="a b"; export CHILD=$V; echo "$CHILD"; readonly LOCK=$V; echo "$LOCK"', []),
    ("{ echo group; echo second; } > compound-output; cat < compound-output | cat > pipeline-output; cat pipeline-output", []),
    ("eval 'echo evaluated'; echo after", []),
]

def invoke(executable, args, cwd):
    return subprocess.run([str(executable), *args], cwd=cwd, capture_output=True, timeout=10)

with tempfile.TemporaryDirectory(prefix="sshl-phase2-") as directory:
    for source, args in CASES:
        # Explicit $0 makes differences in executable names irrelevant.
        args = args or ["named"]
        actual = invoke(SHELL, ["-c", source, *args], directory)
        expected = invoke(REFERENCE, ["-c", source, *args], directory)
        assert (actual.returncode, actual.stdout) == (expected.returncode, expected.stdout), (source, actual, expected)
    script = pathlib.Path(directory) / "script.sh"
    script.write_text('#!/bin/sh\n# comment\n\necho "$0:$#"\nfor x in "$@"; do echo "<$x>"; done\nexit 17\n')
    args = [str(script), "one two", ""]
    actual = invoke(SHELL, args, directory)
    expected = invoke(REFERENCE, args, directory)
    assert (actual.returncode, actual.stdout) == (expected.returncode, expected.stdout)
    sourced = pathlib.Path(directory) / "sourced.sh"
    sourced.write_text('VALUE=sourced\nreturn 7\necho bad\n')
    source = '. ./sourced.sh; echo "$?:$VALUE"'
    assert invoke(SHELL, ["-c", source], directory).stdout == invoke(REFERENCE, ["-c", source], directory).stdout
    for syntax in ('echo $(true)', 'echo $((1+2))', 'echo *.txt', 'echo ${VALUE:-fallback}'):
        assert invoke(SHELL, ["-c", syntax], directory).returncode == 2
    assert invoke(SHELL, ["-c"], directory).returncode == 2
    assert invoke(SHELL, ["missing.sh"], directory).returncode != 0
print(f"PASS: {len(CASES)} dash comparisons, script arguments, source, explicit unsupported syntax, entry-point errors")
