#!/bin/bash
# Code metrics script for SunlightOS Kernel

set -e

echo "=== SunlightOS Kernel Code Metrics ==="
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

# --- Overall metrics ---------------------------------------------------------

read -r rust_total rust_nonblank <<< "$(count_text_lines "*.rs")"
print_line_stats "Rust Code" "$rust_total" "$rust_nonblank"

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

echo "All Tracked Files (Rust, TOML, Markdown, Word, Shell):"
echo "  Total lines:        $all_total"
echo "  Non-blank lines:    $all_nonblank"
echo

echo "Breakdown by file type:"
for ext in rs toml md docx sh; do
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
tracked_count=$(
    {
        find_files "*.rs"
        find_files "*.toml"
        find_files "*.md"
        find_files "*.sh"
        find_files "*.docx"
    } | sort -u | wc -l
)

echo "File counts:"
echo "  Kernel Rust files:  $kernel_files"
echo "  Non-kernel Rust:    $nonkernel_files"
echo "  Rust files total:   $rust_count"
echo "  Cargo manifests:    $cargo_all_count (kernel: $cargo_kernel_count, non-kernel: $cargo_nonkernel_count)"
echo "  Word documents:     $docx_files"
echo "  All tracked files:  $tracked_count"