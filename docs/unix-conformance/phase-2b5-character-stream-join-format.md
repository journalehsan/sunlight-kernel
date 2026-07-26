# Phase 2B.5 — character translation, stream composition, relational join, and formatted output

This phase adds maintained `tr`, `paste`, `join`, and `printf` binaries. It
does not claim POSIX or UNIX certification. The implementation policy is the
SunlightOS POSIX/C single-byte locale; unsupported locale-sensitive behavior
is recorded below instead of being approximated.

## Pre-edit repository trace

| Utility | Existing form | Required foundations | Reusable code | Blockers | Planned action |
|---|---|---|---|---|---|
| `tr` | No maintained binary, built-in, fixture, or image mapping | bounded byte stream; checked arrays; escape/range parsing | `cut` numeric validation and `NativeIo` | no locale/multibyte runtime | add standalone C-locale byte transformer |
| `paste` | No maintained binary or fixture | bounded line readers; multi-input state; delimiter decoder | `comm`/`cat` I/O and cleanup patterns | no shared general line reader | add bounded parallel and serial composer |
| `join` | No maintained binary or fixture | fields; ordered merge; duplicate groups; output list | `compare::byte_cmp`, `sort`/`comm` ordering policy | no locale collation service | add C-locale streaming equality join |
| `printf` | No maintained standalone utility or maintained formatter | format parser; checked integer conversion; chunked padding | libc `write_all`, `cut` numeric parsing | no maintained float formatter | add utility-specific non-variadic formatter |

The old `sunlight-utils/src/main.rs` multicall path was not reused: maintained
utilities enter through the standalone `sunlight-libc::crt0` path.

## Authoritative specification trace

The primary references are the Issue 8 utility specifications:

- [`tr`](https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/tr.html)
- [`paste`](https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/paste.html)
- [`join`](https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/join.html)
- [`printf`](https://pubs.opengroup.org/onlinepubs/9799919799.2024edition/utilities/printf.html)

The relevant definitions are the Issue 8 Base Definitions for utility syntax,
text files and lines, portable character set, character classes, collating
symbols/equivalence classes, locale categories, escape sequences, integer
constants, and formatted output. The authoritative pages were traced before
editing; exact Issue 8 wording was not reproduced here.

### Required forms and policy

| Utility | Synopsis and implemented forms | Input/output and status | Locale/unspecified behavior |
|---|---|---|---|
| `tr` | `tr [-Ccs] string1 [string2]`; literals, escapes, octal, ranges, bracket literals, C classes, deletion, squeeze, complement | stdin only; stdout transformed bytes; syntax errors 2, I/O errors 1 | C-locale bytes are deterministic; locale collation, multibyte character ranges, equivalence and multi-byte collating elements are not implemented; short `string2` uses a deterministic last-character extension where POSIX leaves it unspecified |
| `paste` | `paste [-s] [-d list] [file...]`; parallel/serial, cycling delimiters, escaped delimiters, `-` | files or stdin; output errors/read/open errors 1; syntax errors 2 | line limit is explicit; no synthetic final newline in serial mode; descriptor cap is explicit |
| `join` | `join [-a file_number\| -v file_number] [-e string] [-o list] [-t char] [-1 field] [-2 field] file1 file2`; default and explicit fields, output list, unmatched records, replacement | two ordered files, one may be stdin; both stdin rejected; errors 1, syntax errors 2 | comparison is exactly current bytewise C-locale `compare::byte_cmp`; unsorted-input behavior follows the standard's unspecified/ordered-input contract and is not auto-corrected |
| `printf` | utility format with `%%`, `%b`, `%c`, `%d`, `%i`, `%o`, `%s`, `%u`, `%x`, `%X`; flags, width, precision, escapes, reuse | argv only; malformed format/numeric input non-zero; stdout failure non-zero | C-locale bytes and `.` policy; floating conversions are rejected because no maintained floating formatter exists |

`printf` reuses the format while arguments remain, supplies empty/zero values
for missing operands according to conversion kind, and stops safely for a
zero-consumption format. Width and precision are checked and padding is emitted
in chunks. `%b` has a separate escape decoder and honors `\\c`.

## Conformance ledger

| Utility | Required forms | Implemented | Tested | Known deviations / certification gaps |
|---|---|---|---|---|
| `tr` | `-C/-c/-d/-s`; one/two arrays; C classes; ranges; escapes; deletion/squeeze/translation | yes for bounded C-locale byte forms | parser and binary-stream table tests | no full locale classes/collation/multibyte semantics; fixed 256-byte map; unsupported malformed/locale forms are rejected |
| `paste` | parallel, serial, `-d`, stdin, `-`, unequal files, final-line rules | yes within line/input bounds | unequal parallel, serial no-newline, delimiter cycling | maximum 8 active inputs and 4096-byte logical lines; repeated stdin behavior and descriptor exhaustion are not verified on target |
| `join` | ordered merge, `-1/-2`, `-a/-v`, `-e`, `-o`, `-t`, duplicate Cartesian groups | yes within bounded groups/lines | duplicate Cartesian and structural output-list tests | maximum 8 records per equal-key group; locale collation and unsorted-input diagnostics are not implemented/verified |
| `printf` | literal, escapes, integer/string/character forms, flags, width, precision, `%b`, reuse | yes for non-floating forms | reuse, padding, `%b` stop, zero-consumption tests | floating conversions, locale radix, and exhaustive Issue 8 numeric edge matrices remain gaps |

## Locale and multibyte policy

| Locale-sensitive behavior | Utility | Current support | Gap |
|---|---|---|---|
| `LC_CTYPE` classes and multibyte characters | `tr` | POSIX/C byte classes only | no locale database or UTF-8 decoder; UTF-8 is preserved as bytes |
| `LC_COLLATE` keys/equivalence | `join` | bytewise order shared with `sort`/`comm` | no locale collation elements/equivalence classes |
| `LC_NUMERIC` radix | `printf` | C `.` / integer forms | no floating conversion or locale radix |
| `LC_MESSAGES`, `LANG`, `NLSPATH` | all | diagnostics are fixed English bytes | localization was not implemented or verified |

## Bounds and cleanup policy

| Resource bound | Utility | Value/policy | Verified |
|---|---|---|---|
| translation map/array | `tr` | 256 byte entries; array expansion checked against 256 | host unit tests; target run not verified |
| active inputs/current line | `paste` | 8 readers; 4096 content bytes plus newline carry; no silent truncation | host unit tests; target descriptor-failure injection not verified |
| duplicate groups/current records | `join` | 8 records per side; Cartesian pairs emitted in encounter order | host duplicate-group test; oversized-group target path not verified |
| format/padding | `printf` | 1024-byte escaped argument scratch; chunked padding; checked width/precision | host parser tests; allocation-failure injection not verified |

All four binaries use the maintained startup, `sunlight-libc` I/O, structured
exit path, and panic reporting. They do not open KV or directory-mutation
capabilities. Image embedding and target ELF inspection remain build-gate
evidence, not source-only claims.

## Fixtures and acceptance

The RAMFS test overlay adds `/tests/paste-a`, `/tests/paste-b`,
`/tests/paste-serial`, `/tests/join-a`, and `/tests/join-b`. `tools/test.sh
phase2b5` uses the existing keyboard-injection feature and default development
image. It exercises stdin `tr` translation/deletion, parallel and serial
`paste`, matching and unpaired `join`, integer and escaped `printf`, and an
invalid numeric argument. The image built and booted, but the gate did not
reach command-exit markers because the cold-boot run locked the login before
the injected shell sequence. Command-level target acceptance is therefore not
verified; the failure is retained rather than weakening the expectation file.

## Unverified cases and remaining certification gaps

Not verified here: full Issue 8 option/operand matrices; locale changes through
`LC_ALL`, `LC_CTYPE`, `LC_COLLATE`, and `LC_NUMERIC`; malformed UTF-8 policy on
the real target; partial-write/read fault injection; descriptor exhaustion;
allocation limits; oversized duplicate groups; exact target diagnostics;
target image manifests; and full previous-utility regression and cleanup
diagnostics. These omissions prevent a claim of utility or system conformance.

Recommended Phase 2B.6 candidates are `unexpand`, `nl`, and `tee`, subject to
the same repository/specification trace and explicit resource bounds.
