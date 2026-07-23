# SunlightOS Release Naming and Versioning Charter

This document records the permanent release identity, naming conventions, and
versioning principles of SunlightOS. It exists so that every future maintainer,
regardless of when or how they join the project, can carry forward the same
system.

## 1. Astronomical Release Naming

SunlightOS major stable generations are named after the Solar System. A new
major stable generation is planned every 30 months, in this order:

1. Mercury
2. Venus
3. Earth
4. Mars
5. Jupiter
6. Saturn
7. Uranus
8. Neptune

The first stable generation will be:

> **SunlightOS 1.0 “Mercury”**

The project epoch is `2026-06-06`; the automatic Mercury boundary is
`2028-12-06`, with later planetary boundaries following every 30 months.

Earth represents the intended stage at which SunlightOS has grown beyond a
stable operating system into a broad, mature, and self-sustaining ecosystem
spanning hardware, applications, services, development tools, compatibility
environments, and local intelligence. Mercury remains the first stable
generation intended for regular daily use on supported systems.

After Neptune, the naming tradition must continue with moons of the Solar
System. The exact moon ordering will be defined in a future revision of this
charter before it is needed. When that revision is made, it must respect the
same astronomical-naming ethos, choosing prominent and widely recognised moons
in a deliberate order that honours the Solar System.

Future maintainers must not discard, reorder, or replace this astronomical
naming tradition for marketing reasons or for personal preference. Planetary
and lunar names are part of the SunlightOS identity and outlast any single
contributor or generation of the project.

## 2. Automatic Version Progression

Automatic version progression is a permanent and non-negotiable SunlightOS
principle.

SunlightOS version numbers are calculated automatically. A maintainer must
never manually decide or announce that the codebase has suddenly become an
arbitrary version.

The rules below are binding for all release activity:

* Version progression must result from the repository’s defined version
  algorithm and measurable project changes. It must not originate from a human
  judgement call about how the version “feels.”

* Git tags, package metadata, release pages, boot strings, and all
  human-facing version strings must reflect the calculated version. They must
  not independently define or override it.

* A release manager may decide whether a particular build is ready to publish.
  That decision must never include inventing or manually bumping its version
  number.

* The automatic algorithm must remain inspectable, reproducible, documented,
  and version-controlled within the repository. Any maintainer should be able
  to run the same algorithm against the same repository state and algorithm
  revision and obtain the same result.

* Changes to the versioning algorithm itself require an explicit, documented
  technical decision with a clear rationale. Such changes must never be used
  merely to manufacture a preferred version number or to skip ahead in the
  release sequence.

* No individual crate, application, or service within the SunlightOS workspace
  should establish an independent SunlightOS system version. Sub-component
  versions are separate from the system version and must not be conflated with
  it.

* Manual overrides must never alter the canonical SunlightOS system version.
  An emergency build may carry separately documented revision or build metadata
  where the version format permits it, but its underlying system version must
  remain the result of the automatic algorithm.

* If automatic version calculation fails for any reason, tooling must report
  the failure clearly. It must not silently fall back to a manually chosen
  version or to a stale cached value.

## 3. Future Weighted Versioning

The automatic versioning algorithm may later evolve from its current
calculation into a weighted change model. In such a model, foundational
changes would carry greater version significance than superficial changes.

Areas that may naturally receive higher weight include:

* Kernel and memory-management changes
* IPC and capability-model changes
* ABI and system-call changes
* Security and isolation changes
* Driver and hardware-support changes
* Filesystem, networking, and compatibility-runtime changes

Areas that may receive different or lower weights include:

* Individual application or userland-tool changes
* Documentation and formatting changes
* Generated files and build artefacts
* Raw line-count growth in non-critical paths

Lines of code are a useful historical metric and should continue to be tracked.
However, raw line count alone must never permanently determine the semantic
importance of a change. Weighted versioning is intended to supplement line
metrics with structural awareness, not to discard them.

This section records a direction, not an implementation. The exact weights,
categories, and thresholds require a separate technical design, discussion, and
implementation cycle. When that work occurs, it must respect the automatic
version progression principles in Section 2 and be documented in a revision of
this charter.

## 4. Continuity

Future maintainers of SunlightOS inherit more than the source code. They
inherit a release identity and a set of traditions that give the project its
shape across decades.

The astronomical naming scheme ties each generation to something larger than
the project itself. The automatic versioning principle keeps release numbers
honest and reproducible. The weighted-versioning direction points toward a
system in which the version number reflects the structural and system-wide
significance of changes more accurately.

These commitments exist so that SunlightOS can remain a coherent, long-lived
project even as contributors come and go. Maintainers are expected to uphold
them as part of their stewardship of the project.
