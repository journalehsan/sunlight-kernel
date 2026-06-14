#!/bin/bash
# Code metrics script for SunlightOS Kernel

set -e

echo "=== SunlightOS Kernel Code Metrics ==="
echo

# Helper function to count lines
count_lines() {
    local pattern="$1"
    local description="$2"

    local total=$(find . -type f -name "$pattern" ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
    local nonblank=$(find . -type f -name "$pattern" ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs grep -h . 2>/dev/null | wc -l)

    echo "$description:"
    echo "  Total lines:        $total"
    echo "  Non-blank lines:    $nonblank"
    echo
}

# Helper function to count lines in a specific path
count_lines_path() {
    local path="$1"
    local pattern="$2"
    local description="$3"

    local total=$(find "$path" -type f -name "$pattern" ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
    local nonblank=$(find "$path" -type f -name "$pattern" ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs grep -h . 2>/dev/null | wc -l)

    if [ "$total" = "" ] || [ "$total" = "0" ]; then
        total="0"
        nonblank="0"
    fi

    echo "$description:"
    echo "  Total lines:        $total"
    echo "  Non-blank lines:    $nonblank"
    echo
}

# Count Rust files
count_lines "*.rs" "Rust Code"

# Count all code files
total_all=$(find . -type f \( -name "*.rs" -o -name "*.toml" -o -name "*.md" -o -name "*.sh" \) ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
nonblank_all=$(find . -type f \( -name "*.rs" -o -name "*.toml" -o -name "*.md" -o -name "*.sh" \) ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs grep -h . 2>/dev/null | wc -l)

echo "All Code Files (Rust, TOML, Markdown, Shell):"
echo "  Total lines:        $total_all"
echo "  Non-blank lines:    $nonblank_all"
echo

# Breakdown by file type
echo "Breakdown by file type:"
for ext in rs toml md sh; do
    count=$(find . -type f -name "*.$ext" ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}' || echo "0")
    echo "  .$ext files:       $count"
done
echo

# Microkernel Analysis: Kernel vs Non-Kernel Code
echo "=== Microkernel Analysis ==="
echo

# Kernel Rust code
echo "Kernel Code (kernel/ folder):"
kernel_total=$(find ./kernel -type f -name "*.rs" ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
kernel_nonblank=$(find ./kernel -type f -name "*.rs" ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs grep -h . 2>/dev/null | wc -l)

if [ "$kernel_total" = "" ] || [ "$kernel_total" = "0" ]; then
    kernel_total="0"
    kernel_nonblank="0"
fi

echo "  Total lines:        $kernel_total"
echo "  Non-blank lines:    $kernel_nonblank"
echo

# Non-Kernel Rust code: every top-level dir except kernel/ and hidden dirs
nonkernel_dirs=$(find . -maxdepth 1 -mindepth 1 -type d ! -name 'kernel' ! -name 'target' ! -name '.*' | sort)

echo "Non-Kernel Code ($(echo $nonkernel_dirs | sed 's|\./||g; s/ /\/, /g')/ folders):"
nonkernel_total=$(find $nonkernel_dirs -type f -name "*.rs" ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}')
nonkernel_nonblank=$(find $nonkernel_dirs -type f -name "*.rs" ! -path './target/*' ! -path './.git/*' 2>/dev/null | xargs grep -h . 2>/dev/null | wc -l)

if [ "$nonkernel_total" = "" ] || [ "$nonkernel_total" = "0" ]; then
    nonkernel_total="0"
    nonkernel_nonblank="0"
fi

echo "  Total lines:        $nonkernel_total"
echo "  Non-blank lines:    $nonkernel_nonblank"
echo

# Calculate ratios
rust_count=$(find . -type f -name "*.rs" ! -path './target/*' ! -path './.git/*' 2>/dev/null | wc -l)
kernel_files=$(find ./kernel -type f -name "*.rs" ! -path './target/*' ! -path './.git/*' 2>/dev/null | wc -l)
nonkernel_files=$(find $nonkernel_dirs -type f -name "*.rs" ! -path './target/*' ! -path './.git/*' 2>/dev/null | wc -l)
total_count=$(find . -type f \( -name "*.rs" -o -name "*.toml" -o -name "*.md" -o -name "*.sh" \) ! -path './target/*' ! -path './.git/*' 2>/dev/null | wc -l)

echo "Microkernel Ratio:"
if [ "$kernel_total" -gt 0 ] && [ "$nonkernel_total" -gt 0 ]; then
    ratio=$(echo "scale=2; $nonkernel_total / $kernel_total" | bc)
    echo "  Non-kernel:kernel ratio: $ratio:1 (by non-blank lines: $(echo "scale=2; $nonkernel_nonblank / $kernel_nonblank" | bc):1)"
fi
echo

echo "File counts:"
echo "  Kernel files:       $kernel_files"
echo "  Non-kernel files:   $nonkernel_files"
echo "  Rust files total:   $rust_count"
echo "  All tracked files:  $total_count"
