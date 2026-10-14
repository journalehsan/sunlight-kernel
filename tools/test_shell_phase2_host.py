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
    ('NAME=Sunlight; export NAME; sh -c \'echo "$NAME"\'; NAME=temporary sh -c \'echo "$NAME"\'; echo "$NAME"; unset NAME; sh -c \'echo "<$NAME>"\'', []),
    ('NAME=local; NAME=temporary sh -c \'echo "$NAME"\'; echo "$NAME"; sh -c \'echo "<$NAME>"\'', []),
    ('NAME=permanent :; echo "$NAME"; NAME=changed export NAME; sh -c \'echo "$NAME"\'; NAME=modified eval \'NAME=final\'; echo "$NAME"', []),
    ('export NAME; /usr/bin/printenv NAME; echo $?; NAME=assigned; /usr/bin/printenv NAME; unset NAME; /usr/bin/printenv NAME; echo $?', []),
    ('readonly NAME=locked; NAME=bad sh -c \'echo bad\'; echo unreachable', []),
    ('NAME=parent; export NAME; NAME=temp /missing-sshl-command || :; sh -c \'echo "$NAME"\'', []),
    ('NAME=parent; export NAME; (NAME=child; sh -c \'echo "$NAME"\'); sh -c \'echo "$NAME"\'', []),
    ('A=one B=$A sh -c \'echo "$A:$B"\'; echo "<$A:$B>"', []),
    ('NAME=parent; export NAME; NAME=left sh -c \'echo "$NAME"\' | NAME=right sh -c \'echo "$NAME"; cat\'; echo "$NAME"', []),
]

SUBSTITUTIONS = [
    'echo "$(echo hello)"',
    'value=$(printf "SunlightOS\\n"); echo "$value"',
    'echo "$(echo "$(echo nested)")"',
    "echo '$(echo literal)'",
    'echo "\\$(echo literal)"',
    "value=$(printf 'hello\\n\\n'); printf '<%s>\\n' \"$value\"",
    "value=$(printf 'a\\nb\\n'); printf '<%s>\\n' \"$value\"",
    "printf '<%s>\\n' \"$(true)\"; set -- $(true); echo $#",
    "set -- $(printf 'a b\\nc'); printf '<%s>\\n' \"$@\"",
    "set -- pre$(printf 'a b')post; printf '<%s>\\n' \"$@\"",
    "IFS=:; set -- $(printf 'a::b'); printf '<%s>\\n' \"$@\"",
    "f() { printf '%s\\n' \"$1\"; return 7; }; v=$(f 'a b'); echo \"$?:$v\"",
    'echo "$(if true; then echo yes; else echo bad; fi)"',
    'echo "$(for x in a b; do echo "$x"; done)"',
    "echo \"$(case x in x) echo ')';; *) echo bad;; esac)\"",
    "echo \"$(f() { echo ')'; }; f)\"",
    'echo "$( (echo sub); { echo group; } )"',
    'echo "$(echo first\necho second\n)"',
    'echo "$(echo hello # ) ignored\n)"',
    'NAME=parent; v=$(NAME=child; cd /tmp; pwd; echo "$NAME"); echo "$v:$NAME"; pwd',
    'export NAME=visible; v=$(sh -c \'echo "$NAME"\'); echo "$v"',
    'NAME=parent; v=$(NAME=temp sh -c \'echo "$NAME"\'); echo "$v:$NAME"',
    'A=one B=$(echo "$A"); echo "$A:$B"',
    'NAME=parent; NAME=temp echo "$(echo "$NAME")"; echo "$NAME"',
    'v=$(false); echo "$?"; echo "$(false)"; echo "$?"',
    'a=$(false) b=$(true); echo $?; a=$(true) b=$(false); echo $?',
    'v=$(false | true); echo $?; v=$(true | false); echo $?',
    'v=$(echo "$(false)"); echo $?; v=$(exit 17); echo $?',
    'echo "$(echo hello | grep hello | wc -c)"',
    'v=$(echo out; echo err >&2); echo "$v"',
    'v=$(echo hidden > substitution-file; cat substitution-file); echo "$v"',
    'v=$(echo err >&2; echo out) 2> substitution-error; echo "$v"; cat substitution-error',
    'echo out > "$(echo substitution-path)"; cat substitution-path',
    "v=$(printf '$NAME `echo no` \\n'); printf '%s\\n' \"$v\"",
    'for x in a b c; do echo "$(echo "$x" | cat)"; done',
    "v=$(head -c 8192 /dev/zero | tr '\\000' x); printf '%s' \"$v\" | wc -c",
    "v=$(yes x | head -c 8192); printf '%s' \"$v\" | wc -c",
    'v=$(echo out; printf err >&2); echo "$v"',
    'echo "$(echo "$(printf err >&2; printf inner)")"',
    'echo "$(echo "$(printf err >&2; printf inner)" 2>&1)"',
    "case abc in $(printf 'a*')) echo yes;; *) echo no;; esac",
    "case abc in \"$(printf 'a*')\") echo no;; *) echo yes;; esac",
    'false; echo "$(true):$?"; echo $?',
    'v=$(); echo "$?:$v"',
    "v=$(printf 'x\\r\\n\\n'); printf '<%s>\\n' \"$v\"",
]
# Stable stderr cases above must match as well as stdout and status. Diagnostic
# wording for syntax, missing executables and failed opens is shell-specific.
DIAGNOSTIC_SUBSTITUTIONS = [
    'v=$(missing-sshl-substitution-command); echo "$?:$v"',
    'echo $(echo',
    'echo "$(echo hello)',
    'echo $(echo |)',
    'echo $(if true; then echo bad)',
    'echo $(case x in x) echo bad;;)',
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
    for source in SUBSTITUTIONS:
        actual = invoke(SHELL, ["-c", source, "named"], directory)
        expected = invoke(REFERENCE, ["-c", source, "named"], directory)
        assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr), (source, actual, expected)
    for source in DIAGNOSTIC_SUBSTITUTIONS:
        actual = invoke(SHELL, ["-c", source, "named"], directory)
        expected = invoke(REFERENCE, ["-c", source, "named"], directory)
        assert (actual.returncode, actual.stdout) == (expected.returncode, expected.stdout), (source, actual, expected)
        assert bool(actual.stderr) == bool(expected.stderr), (source, actual, expected)
    # Retain the native shell's established redirection-error status (1).
    # POSIX requires a nonzero status; dash chooses 2. Assert both explicitly.
    source = 'v=$(echo bad < /no-such-sshl-substitution-file); echo "$?:$v"'
    actual = invoke(SHELL, ["-c", source, "named"], directory)
    expected = invoke(REFERENCE, ["-c", source, "named"], directory)
    assert (actual.returncode, actual.stdout) == (0, b"1:\n")
    assert (expected.returncode, expected.stdout) == (0, b"2:\n")
    assert actual.stderr and expected.stderr
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
    for syntax in ('echo $((1+2))', 'echo *.txt', 'echo ${VALUE:-fallback}'):
        assert invoke(SHELL, ["-c", syntax], directory).returncode == 2
    assert invoke(SHELL, ["-c"], directory).returncode == 2
    assert invoke(SHELL, ["missing.sh"], directory).returncode != 0
print(f"PASS: {len(CASES) + len(SUBSTITUTIONS) + len(DIAGNOSTIC_SUBSTITUTIONS)} dash comparisons ({len(SUBSTITUTIONS) + len(DIAGNOSTIC_SUBSTITUTIONS)} substitution cases), 1 documented redirection-status difference, script arguments, source, unsupported syntax, entry-point errors")
