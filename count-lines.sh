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

# Summary stats
rust_count=$(find . -type f -name "*.rs" ! -path './target/*' ! -path './.git/*' 2>/dev/null | wc -l)
total_count=$(find . -type f \( -name "*.rs" -o -name "*.toml" -o -name "*.md" -o -name "*.sh" \) ! -path './target/*' ! -path './.git/*' 2>/dev/null | wc -l)

echo "File counts:"
echo "  Rust files:         $rust_count"
echo "  All tracked files:  $total_count"
