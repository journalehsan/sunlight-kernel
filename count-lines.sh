#!/bin/bash
# Code metrics script for SunlightOS Kernel

set -e

echo "=== SunlightOS Kernel Code Metrics ==="
echo

# --- Project age (days since first commit) ------------------------------------

# Ordinal suffix for a positive integer (1st, 2nd, 3rd, 4th, 11th, 21st, …).
ordinal_suffix() {
    local n=$1
    local mod100=$((n % 100))
    local mod10=$((n % 10))

    if [ "$mod100" -ge 11 ] && [ "$mod100" -le 13 ]; then
        echo "th"
        return
    fi
    case "$mod10" in
        1) echo "st" ;;
        2) echo "nd" ;;
        3) echo "rd" ;;
        *) echo "th" ;;
    esac
}

# Days since the first git commit (day 1 = commit day). Falls back to 2026-06-06.
project_day_info() {
    local first_date
    first_date=$(git log --reverse --format='%ad' --date=short 2>/dev/null | head -1)

    if [ -z "$first_date" ]; then
        first_date="2026-06-06"
    fi

    local today
    today=$(date +%Y-%m-%d)

    # Inclusive day count: first commit day is day 1 of the project.
    local days
    days=$(( ($(date -d "$today" +%s) - $(date -d "$first_date" +%s)) / 86400 + 1 ))

    if [ "$days" -lt 1 ]; then
        days=1
    fi

    local suffix
    suffix=$(ordinal_suffix "$days")

    echo "$days" "$suffix" "$first_date" "$today"
}

read -r PROJECT_DAY PROJECT_DAY_SUFFIX PROJECT_START PROJECT_TODAY <<< "$(project_day_info)"
echo "Project day:          ${PROJECT_DAY}${PROJECT_DAY_SUFFIX} day of the project"
echo "  Since first commit: $PROJECT_START  (today: $PROJECT_TODAY)"
echo

# Paths excluded from all metrics.
FIND_PRUNE=(
    ! -path './target/*'
    ! -path '*/target/*'
    ! -path './.git/*'
    ! -path './.claude/*'
    ! -path './sunlight-fetch/*.zip'
    ! -path './sunlight-fetch/*.zip?*'
)

# Return newline-separated paths matching a file name pattern.
find_files() {
    local pattern="$1"
    shift
    find . -type f -name "$pattern" "${FIND_PRUNE[@]}" "$@" 2>/dev/null
}

# Count total and non-blank lines across text files.
count_text_lines() {
    local pattern="$1"
    shift

    local files total nonblank
    files=$(find_files "$pattern" "$@")

    if [ -z "$files" ]; then
        echo "0 0"
        return
    fi

    total=$(echo "$files" | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
    nonblank=$(echo "$files" | xargs grep -h . 2>/dev/null | wc -l)

    echo "${total:-0} ${nonblank:-0}"
}

# Count total and non-blank lines across text files under one or more roots.
count_text_lines_in_dirs() {
    local pattern="$1"
    shift
    local dirs=("$@")

    local files total nonblank find_args=()
    for dir in "${dirs[@]}"; do
        find_args+=("$dir")
    done

    files=$(find "${find_args[@]}" -type f -name "$pattern" "${FIND_PRUNE[@]}" 2>/dev/null)

    if [ -z "$files" ]; then
        echo "0 0"
        return
    fi

    total=$(echo "$files" | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
    nonblank=$(echo "$files" | xargs grep -h . 2>/dev/null | wc -l)

    echo "${total:-0} ${nonblank:-0}"
}

# Approximate non-blank text lines inside .docx files.
count_docx_lines() {
    local files total=0

    files=$(find_files "*.docx" "$@")
    if [ -z "$files" ]; then
        echo "0"
        return
    fi

    while IFS= read -r file; do
        local lines
        lines=$(
            unzip -p "$file" word/document.xml 2>/dev/null |
                grep -oE '<w:t[^>]*>[^<]*</w:t>|<w:tab/>|<w:br[^/]*/>' |
                sed -E 's/<w:t[^>]*>([^<]*)<\/w:t>/\1/; s/<w:tab\/>/\n/; s/<w:br[^/]*\/>/\n/' |
                grep -c . || true
        )
        total=$((total + lines))
    done <<< "$files"

    echo "$total"
}

# Count Cargo.toml manifests and their line totals.
count_cargo_manifests() {
    local scope="$1"
    local files count total

    case "$scope" in
        all)
            files=$(find_files "Cargo.toml")
            ;;
        kernel)
            files=$(find_files "Cargo.toml" -path './kernel/*')
            ;;
        nonkernel)
            files=$(find . -type f -name "Cargo.toml" "${FIND_PRUNE[@]}" ! -path './kernel/*' 2>/dev/null)
            ;;
        *)
            echo "0 0"
            return
            ;;
    esac

    if [ -z "$files" ]; then
        echo "0 0"
        return
    fi

    count=$(echo "$files" | wc -l)
    total=$(echo "$files" | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
    echo "$count ${total:-0}"
}

print_line_stats() {
    local label="$1"
    local total="$2"
    local nonblank="$3"

    echo "$label:"
    echo "  Total lines:        $total"
    echo "  Non-blank lines:    $nonblank"
    echo
}

# --- Git-based helpers for DOS guest sources (tracked files only) -----------

# List tracked files whose basename matches the case-insensitive regex.
# Uses git ls-files so that only committed files are considered and
# build artefacts (even if tracked like *.exe) are filtered by extension.
list_git_files() {
    local pattern="$1"
    git ls-files 2>/dev/null | grep -iE "$pattern" || true
}

# Given newline-separated file list, return "fcount total_lines nonblank_lines".
# Safe for filenames containing spaces; does not rely on xargs splitting.
count_lines_safe() {
    local files="$1"

    if [ -z "$files" ]; then
        echo "0 0 0"
        return
    fi

    local fcount=0 total=0 nonb=0
    while IFS= read -r f || [ -n "$f" ]; do
        [ -z "$f" ] && continue
        if [ -f "$f" ]; then
            fcount=$((fcount + 1))
            local l
            l=$(wc -l < "$f" 2>/dev/null || echo 0)
            total=$((total + ${l:-0}))
            local nb
            nb=$(grep -c . "$f" 2>/dev/null || echo 0)
            nonb=$((nonb + ${nb:-0}))
        fi
    done <<< "$files"

    echo "$fcount $total $nonb"
}

# Top-level project folders used for per-crate breakdown.
mapfile -t TOP_LEVEL_DIRS < <(
    find . -maxdepth 1 -mindepth 1 -type d \
        ! -name 'target' ! -name '.*' |
        sort
)

mapfile -t NONKERNEL_DIRS < <(
    find . -maxdepth 1 -mindepth 1 -type d \
        ! -name 'kernel' ! -name 'target' ! -name '.*' |
        sort
)

NONKERNEL_DIR_NAMES=""
for dir in "${NONKERNEL_DIRS[@]}"; do
    name=${dir#./}
    if [ -n "$NONKERNEL_DIR_NAMES" ]; then
        NONKERNEL_DIR_NAMES+=", "
    fi
    NONKERNEL_DIR_NAMES+="$name"
done

# Compute DOS guest source metrics (Pascal + Assembly + DOS Batch) using git.
# This ensures only tracked files, case-insensitive exts, and no binaries.
pas_files=$(list_git_files '\.(pas|pp|p|inc)$')
read -r pas_fcount pas_total pas_nonblank <<< "$(count_lines_safe "$pas_files")"

asm_files=$(list_git_files '\.(asm|s|S)$')
read -r asm_fcount asm_total asm_nonblank <<< "$(count_lines_safe "$asm_files")"

bat_files=$(list_git_files '\.bat$')
read -r bat_fcount bat_total bat_nonblank <<< "$(count_lines_safe "$bat_files")"

guest_fcount=$((pas_fcount + asm_fcount + bat_fcount))
guest_total=$((pas_total + asm_total + bat_total))
guest_nonblank=$((pas_nonblank + asm_nonblank + bat_nonblank))

# --- Overall metrics ---------------------------------------------------------

read -r rust_total rust_nonblank <<< "$(count_text_lines "*.rs")"
print_line_stats "Rust Code" "$rust_total" "$rust_nonblank"

# --- DOS Guest Source Code (Free Pascal + related DOS guest sources) ---------

echo "DOS Guest Source Code:"
echo

echo "  Pascal:"
echo "    Files:              $pas_fcount"
echo "    Total lines:        $pas_total"
echo "    Non-blank lines:    $pas_nonblank"
echo

echo "  Assembly:"
echo "    Files:              $asm_fcount"
echo "    Total lines:        $asm_total"
echo "    Non-blank lines:    $asm_nonblank"
echo

echo "  DOS Batch:"
echo "    Files:              $bat_fcount"
echo "    Total lines:        $bat_total"
echo "    Non-blank lines:    $bat_nonblank"
echo

echo "  Guest source total:"
echo "    Files:              $guest_fcount"
echo "    Total lines:        $guest_total"
echo "    Non-blank lines:    $guest_nonblank"
echo

read -r md_total md_nonblank <<< "$(count_text_lines "*.md")"
docx_files=$(find_files "*.docx" | wc -l)
docx_nonblank=$(count_docx_lines)

echo "Documentation (Markdown + Word):"
echo "  Markdown lines:     $md_total (non-blank: $md_nonblank)"
echo "  Word .docx files:   $docx_files (approx non-blank lines: $docx_nonblank)"
echo "  Doc total lines:    $((md_total + docx_nonblank)) (non-blank: $((md_nonblank + docx_nonblank)))"
echo

tracked_text_files=$(
    {
        find_files "*.rs"
        find_files "*.toml"
        find_files "*.md"
        find_files "*.sh"
    } | sort -u
)

if [ -n "$tracked_text_files" ]; then
    all_total=$(echo "$tracked_text_files" | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
    all_nonblank=$(echo "$tracked_text_files" | xargs grep -h . 2>/dev/null | wc -l)
else
    all_total="0"
    all_nonblank="0"
fi
all_nonblank=$((all_nonblank + docx_nonblank))
all_total=$((all_total + docx_nonblank))

# Incorporate DOS guest sources (Pascal/Assembly/Batch) into the aggregate tracked
# source lines. These were obtained via git ls-files so no double-counting with
# the find-based rs/toml/etc lists above.
all_total=$((all_total + guest_total))
all_nonblank=$((all_nonblank + guest_nonblank))

echo "All Tracked Files (Rust, Pascal, Assembly, Batch, TOML, Markdown, Word, Shell):"
echo "  Total lines:        $all_total"
echo "  Non-blank lines:    $all_nonblank"
echo

echo "Breakdown by file type:"
# .rs first (use already-computed to avoid recompute and keep identical value)
echo "  .rs files:       ${rust_total:-0}"
echo "  Pascal files:    ${pas_total:-0} lines across ${pas_fcount:-0} file(s)"
echo "  Assembly files:  ${asm_total:-0} lines across ${asm_fcount:-0} file(s)"
echo "  .bat files:      ${bat_total:-0} lines across ${bat_fcount:-0} file(s)"
for ext in toml md docx sh; do
    if [ "$ext" = "docx" ]; then
        echo "  .$ext files:       $docx_nonblank lines across $docx_files file(s)"
    else
        count=$(find_files "*.$ext" | xargs -r wc -l 2>/dev/null | tail -1 | awk '{print $1}' || echo "0")
        echo "  .$ext files:       ${count:-0}"
    fi
done
echo

# --- Per top-level folder ----------------------------------------------------

echo "=== Per Top-Level Folder (Rust) ==="
echo

for dir in "${TOP_LEVEL_DIRS[@]}"; do
    read -r dir_total dir_nonblank <<< "$(count_text_lines "*.rs" -path "$dir/*")"
    name=${dir#./}
    printf "  %-22s %6s lines (%s non-blank)\n" "$name/" "$dir_total" "$dir_nonblank"
done
echo

# Per top-level for Pascal sources only (using git-discovered files).
# Only directories that contain Pascal files are shown (keeps output readable).
# If no Pascal files exist anywhere, the section is emitted empty but valid.
echo "=== Per Top-Level Folder (Pascal) ==="
echo
if [ "${pas_fcount:-0}" -gt 0 ]; then
    mapfile -t PAS_PASCAL_TOP < <(
        echo "$pas_files" | sed 's|/.*||' | sort -u
    )
    for d in "${PAS_PASCAL_TOP[@]}"; do
        dir_pas_files=$(echo "$pas_files" | grep -E "^${d}/" || true)
        read -r df dt dn <<< "$(count_lines_safe "$dir_pas_files")"
        printf "  %-22s %6s lines (%s non-blank, %s file(s))\n" "$d/" "$dt" "$dn" "$df"
    done
fi
echo

# --- Cargo workspace metrics -------------------------------------------------

echo "=== Cargo Workspace Metrics ==="
echo

read -r cargo_all_count cargo_all_lines <<< "$(count_cargo_manifests all)"
read -r cargo_kernel_count cargo_kernel_lines <<< "$(count_cargo_manifests kernel)"
read -r cargo_nonkernel_count cargo_nonkernel_lines <<< "$(count_cargo_manifests nonkernel)"

echo "All Cargo.toml manifests:"
echo "  Manifest count:     $cargo_all_count"
echo "  Total lines:        $cargo_all_lines"
echo

echo "Kernel Cargo.toml (kernel/):"
echo "  Manifest count:     $cargo_kernel_count"
echo "  Total lines:        $cargo_kernel_lines"
echo

echo "Non-kernel Cargo.toml ($NONKERNEL_DIR_NAMES):"
echo "  Manifest count:     $cargo_nonkernel_count"
echo "  Total lines:        $cargo_nonkernel_lines"
echo

# --- Microkernel analysis ----------------------------------------------------

echo "=== Microkernel Analysis ==="
echo

read -r kernel_total kernel_nonblank <<< "$(count_text_lines "*.rs" -path './kernel/*')"
print_line_stats "Kernel Rust (kernel/)" "$kernel_total" "$kernel_nonblank"

read -r nonkernel_total nonkernel_nonblank <<< "$(count_text_lines_in_dirs "*.rs" "${NONKERNEL_DIRS[@]}")"
print_line_stats "Non-kernel Rust ($NONKERNEL_DIR_NAMES)" "$nonkernel_total" "$nonkernel_nonblank"

echo "Rust microkernel ratio:"
if [ "$kernel_total" -gt 0 ] && [ "$nonkernel_total" -gt 0 ]; then
    rust_ratio=$(echo "scale=2; $nonkernel_total / $kernel_total" | bc)
    rust_nonblank_ratio=$(echo "scale=2; $nonkernel_nonblank / $kernel_nonblank" | bc)
    echo "  Non-kernel:kernel:  ${rust_ratio}:1 (lines), ${rust_nonblank_ratio}:1 (non-blank)"
fi
echo

echo "Cargo crate microkernel ratio:"
if [ "$cargo_kernel_count" -gt 0 ] && [ "$cargo_nonkernel_count" -gt 0 ]; then
    crate_ratio=$(echo "scale=2; $cargo_nonkernel_count / $cargo_kernel_count" | bc)
    crate_line_ratio=$(echo "scale=2; $cargo_nonkernel_lines / $cargo_kernel_lines" | bc)
    echo "  Non-kernel:kernel:  ${crate_ratio}:1 (manifests), ${crate_line_ratio}:1 (Cargo.toml lines)"
fi
echo

rust_count=$(find_files "*.rs" | wc -l)
kernel_files=$(find_files "*.rs" -path './kernel/*' | wc -l)
nonkernel_files=$(find "${NONKERNEL_DIRS[@]}" -type f -name "*.rs" "${FIND_PRUNE[@]}" 2>/dev/null | wc -l)
# Real total of all tracked files in the repository (git ls-files authoritative).
full_tracked_count=$(git ls-files 2>/dev/null | wc -l || echo 0)

echo "File counts:"
echo "  Kernel Rust files:  $kernel_files"
echo "  Non-kernel Rust:    $nonkernel_files"
echo "  Rust files total:   $rust_count"
echo "  Pascal source files: $pas_fcount"
echo "  Assembly source files: $asm_fcount"
echo "  DOS batch files:    $bat_fcount"
echo "  DOS guest source files total: $guest_fcount"
echo "  Cargo manifests:    $cargo_all_count (kernel: $cargo_kernel_count, non-kernel: $cargo_nonkernel_count)"
echo "  Word documents:     $docx_files"
echo "  All tracked files:  $full_tracked_count"

# --- Guest Compatibility Analysis (separate from Rust microkernel ratio) ----

echo "=== Guest Compatibility Analysis ==="
echo

echo "DOS guest Pascal:"
echo "  Total lines:        $pas_total"
echo "  Non-blank lines:    $pas_nonblank"
echo "  Source files:       $pas_fcount"
echo

# Use git ls-files + safe counter for Chronos host Rust (consistent with DOS guest preference for tracked files).
chronos_core_rust=$(git ls-files 2>/dev/null | grep '^chronos-core/.*\.rs$' || true)
read -r _ chronos_core_t chronos_core_nb <<< "$(count_lines_safe "$chronos_core_rust")"
sunlight_chronos_rust=$(git ls-files 2>/dev/null | grep '^sunlight-chronos/.*\.rs$' || true)
read -r _ sunlight_chronos_t sunlight_chronos_nb <<< "$(count_lines_safe "$sunlight_chronos_rust")"
chronos_host_t=$((chronos_core_t + sunlight_chronos_t))
chronos_host_nb=$((chronos_core_nb + sunlight_chronos_nb))

echo "Chronos host/runtime Rust:"
echo "  chronos-core:       $chronos_core_t lines ($chronos_core_nb non-blank)"
echo "  sunlight-chronos:   $sunlight_chronos_t lines ($sunlight_chronos_nb non-blank)"
echo "  Combined:           $chronos_host_t lines ($chronos_host_nb non-blank)"
echo

echo "Guest-to-Chronos ratio:"
if [ "$chronos_host_t" -gt 0 ]; then
    echo "  Pascal guest : Chronos host Rust   ${pas_total}:${chronos_host_t} (lines), ${pas_nonblank}:${chronos_host_nb} (non-blank)"
fi
echo
