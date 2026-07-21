# License report — russh stop-stage audit

## Result

No `russh` dependency set is approved for SunlightOS. Consequently there is no
SunlightOS SSH integration lockfile and no distributable dependency closure to
approve.

The direct package metadata for the examined artifact declares:

| Package | Version | License | Source |
| --- | --- | --- |
| `russh` | 0.62.3 | `Apache-2.0` | crates.io package; repository declared as `https://github.com/warp-tech/russh` |

Apache-2.0 is compatible in principle with a permissive-distribution project
provided its license text and required notices are retained. That observation
does **not** approve the complete transitive closure.

## Why this report is intentionally incomplete

The phase stopped at mandatory-runtime incompatibility before a supported
feature selection could be resolved for `x86_64-unknown-none`. The archive's
embedded lockfile includes development, benchmark, and optional-backend
packages, including build-only and proc-macro packages, rather than the actual
SunlightOS closure. Recording every such package as the runtime license set
would be inaccurate; omitting them would violate the audit policy.

Thus the license status is **unresolved / not admitted**, rather than silently
assuming compatibility. No code integration may proceed under this status.

For the next candidate, perform the complete report only after selecting an
exact supported feature set and lockfile. It must enumerate every direct,
transitive, build, and proc-macro package with name, version, SPDX expression,
repository, copyright/notice files, attribution, and redistribution terms.
