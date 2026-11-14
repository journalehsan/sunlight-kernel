#!/bin/bash
set -u
python3 docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b1/validate_final2.py > docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b1/final2-validation-runner.log 2>&1
python3 docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b1/validate_supplemental.py > docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b1/supplemental-runner.log 2>&1
