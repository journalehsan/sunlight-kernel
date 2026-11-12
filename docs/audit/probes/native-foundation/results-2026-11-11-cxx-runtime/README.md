# Phase 22 runtime evidence

Final acceptance uses `accepted-{2,4}cpu/`, `lifecycle-{2,4}cpu/`,
`accepted-validation.json`, `accepted-runtime-abi/`, and `acceptance-summary.json`.
Earlier driver logs record intermediate attempts; they are not final verdicts.
Objects, ELFs, archives, downloaded source archives, and ISOs stay in ignored
`target/phase22/`. This directory contains text evidence only.

The later local-static guard acceptance is in
`guard-acceptance-2026-11-12/`; its `verify.py` checks 93/93 buildable native
cases on both CPU configurations, six guard cases, lifecycle output, the
configured historical probe, the preserved original LINK_FAIL, and historical
probe-source integrity.

`python3 docs/audit/probes/native-foundation/results-2026-11-11-cxx-runtime/verify.py`
checks exact outputs/statuses, expected termination, 87/87 configured native cases
on both CPU configurations, lifecycle output/allocation witness, fault absence,
and all tracked historical probe sources against HEAD.

The unaugmented historical command remains LINK_FAIL/runtime NOT_RUN; the
unchanged source built with the configured native C++ driver passes in both
guests. See the appended Phase 22 entry in `docs/audit/std-progress.md` for the
full dependency matrix, boundaries, source provenance, and separate failures.
