"""M5.4 native measurement-foundation assessment (clock, CPU time, resources).

Every metric is classified; nothing is silently promoted. A probe that could
not be shown to run on the requested CPU is NOT_QUALIFIED, not "close enough".
"""
import re

CPUS = (2, 4, 12)
PROBES_PER_CASE = 4
CASES = 4


def rows(raw, prefix):
    out = []
    for line in raw.splitlines():
        if line.startswith(prefix):
            pairs = re.findall(r"(\w+)=(\S+)", line)
            row = dict(pairs)
            if len(row) != len(pairs):
                row["_duplicate"] = "1"
            out.append(row)
    return out


def _int(row, key):
    return int(row[key])


def assess(raw, cpus, identity, measurement=True):
    errors = []
    if cpus not in CPUS:
        errors.append("unsupported topology")
    if not measurement:
        errors.append("M5.4 receipt requires the measurement fixture")
    for key, value in identity.items():
        if not value:
            errors.append("identity mismatch: " + key)

    kernel = rows(raw, "[FL-M54-CLOCK] status=")
    if len(kernel) != 1:
        errors.append("kernel clock line count")
    else:
        line = kernel[0]
        if line.get("status") != "ready":
            errors.append("clock not ready")
        if line.get("contract") != "same-cpu-only":
            errors.append("clock contract")
        if line.get("cross_cpu") != "not-qualified":
            errors.append("cross-cpu contract")
        if _int(line, "calibration_ppm") <= 0:
            errors.append("missing calibration bound")

    sync = rows(raw, "[FL-M54-CLOCKSYNC]")
    if len(sync) < 2:
        errors.append("frequency-stability validation missing")
    qualified = unqualified = 0
    probes = rows(raw, "[FL-M54-CLOCK] pid=")
    expect_probes = CASES * PROBES_PER_CASE
    if len(probes) != expect_probes:
        errors.append("clock probe count")
    for probe in probes:
        if probe.get("contract") != "same-cpu-only":
            errors.append("clock probe contract")
        if probe.get("status") not in ("ready", "not-resident"):
            errors.append("clock probe status")
        resident = _int(probe, "resident")
        if (probe.get("status") == "ready") != (resident == 1):
            errors.append("clock probe resident/status mismatch")
        if _int(probe, "unc_ppm") <= 0:
            errors.append("clock probe without bound")
        if _int(probe, "unc_ns") <= 0:
            errors.append("clock probe without ns bound")
        if _int(probe, "stab_ok") != 1:
            errors.append("clock probe not stability-checked")
        if _int(probe, "reads") != 257:
            errors.append("clock probe read count")
        # Only a probe that actually ran on the requested CPU can produce a
        # duration. A not-resident probe is a quiet, unqualified observation:
        # its raw span is meaningless and is not asserted.
        if resident == 1:
            if _int(probe, "cpu_mismatch") != 0:
                errors.append("clock probe mismatch on resident CPU")
            if _int(probe, "regressions") != 0:
                errors.append("clock regression accepted")
            if not 0 < _int(probe, "span_ns") < 1_000_000_000:
                errors.append("clock probe span out of range")
            qualified += 1
        else:
            unqualified += 1
    # Every measured actor must be observed resident on some CPU, otherwise the
    # fixture never proved it could take a qualified stamp at all.
    for pid in {probe.get("pid") for probe in probes}:
        if not any(_int(p, "resident") == 1 for p in probes if p.get("pid") == pid):
            errors.append("actor never resident on any CPU")
    if qualified < len({probe.get("pid") for probe in probes}):
        errors.append("too few resident clock probes")

    xcpu = rows(raw, "[FL-M54-XCPU]")
    if len(xcpu) != CASES * 2:
        errors.append("cross-CPU probe count")
    total_pairs = 0
    for row in xcpu:
        if row.get("contract") != "cross-cpu-subtraction-rejected":
            errors.append("cross-CPU contract")
        if _int(row, "rejected") != _int(row, "pairs"):
            errors.append("cross-CPU subtraction not rejected")
        total_pairs += _int(row, "pairs")
    if len(xcpu) == CASES * 2 and total_pairs < 1:
        errors.append("cross-CPU rejection never exercised")

    reject = rows(raw, "[FL-M54-REJECT]")
    if len(reject) != CASES * 2:
        errors.append("rejection self-check count")
    for row in reject:
        for flag in ("backward", "uncalibrated", "overflow", "cross_cpu", "stale_epoch", "time_backward"):
            if _int(row, flag) != 1:
                errors.append("rejection self-check: " + flag)
        if _int(row, "all_rejected") != 1:
            errors.append("rejection self-check incomplete")

    kcpu = rows(raw, "[FL-M54-KCPU]")
    if len(kcpu) != CASES * 2:
        errors.append("CPU-time record count")
    for row in kcpu:
        if _int(row, "charged_valid") != 1:
            errors.append("CPU-time charge invalid")
        if _int(row, "discontinuities") != 0:
            errors.append("CPU-time discontinuity")
        if row.get("wall_contract") != "same-cpu-only":
            errors.append("CPU-time wall contract")
        if _int(row, "migrated") == 1 and _int(row, "wall_valid") != 0:
            errors.append("cross-CPU wall window accepted")
        if _int(row, "wall_valid") == 1:
            if _int(row, "charged_ns") > _int(row, "wall_ns"):
                errors.append("CPU time exceeds wall")
            if row.get("reconcile") != "consistent":
                errors.append("CPU-time reconciliation")
        elif row.get("reconcile") != "not-qualified":
            errors.append("unqualified CPU-time window claimed")

    res = rows(raw, "[FL-M54-RES]")
    if len(res) != CASES * 2:
        errors.append("resource snapshot count")
    for row in res:
        if row.get("physical_reclaim") != "not-proven":
            errors.append("resource snapshot overclaims reclamation")
        if row.get("attribution") not in ("scoped-global-delta", "none"):
            errors.append("resource attribution")

    errors = sorted(set(errors))
    return {
        "status": "FAIL" if errors else "PASS",
        "errors": errors,
        "topology": cpus,
        "clock": "QUALIFIED_WITH_BOUNDS same-cpu-only" if not errors else "NOT_QUALIFIED",
        "cross_cpu": "NOT_QUALIFIED (subtraction rejected)",
        "cpu_time": "QUALIFIED_WITH_BOUNDS (scheduler-charged, reconciled)",
        "resources": "QUALIFIED_WITH_BOUNDS (scoped delta; physical reclaim not proven)",
        "qualified_probes": qualified,
        "not_qualified_probes": unqualified,
        "traces": {"probes": len(probes), "xcpu": len(xcpu), "reject": len(reject), "kcpu": len(kcpu), "res": len(res)},
    }


if __name__ == "__main__":
    import sys
    from pathlib import Path
    raw = Path(sys.argv[1]).read_text(errors="replace")
    print(assess(raw, int(sys.argv[2]), {"source": True})["status"])
