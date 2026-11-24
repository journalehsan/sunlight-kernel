#!/usr/bin/env python3
"""Exercise the actual Btop ownership wrapper with controlled libc outcomes."""

import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

header = Path(sys.argv[1]).read_text()
wrapper = re.search(r"\bclass IfAddrsPtr \{.*?^\s*\};", header, re.M | re.S)
if wrapper is None:
    raise SystemExit("Could not find Btop's IfAddrsPtr")

fixture = r"""
#include <cassert>
#include <cerrno>
#include <ifaddrs.h>

static ifaddrs owned{};
static ifaddrs sentinel{};
static int calls;
static bool fail;
static bool empty;

int getifaddrs(ifaddrs** out) noexcept {
    *out = fail ? &sentinel : empty ? nullptr : &owned;
    if (!fail) return 0;
    errno = ENETDOWN;
    return -1;
}

void freeifaddrs(ifaddrs* list) noexcept {
    assert(list == &owned);
    ++calls;
}
"""

main = r"""
int main() {
    fail = true;
    { IfAddrsPtr result; assert(result.get_status() == -1); }
    assert(calls == 0);
    fail = false;
    empty = true;
    { IfAddrsPtr result; assert(result.get_status() == 0 && result.get() == nullptr); }
    assert(calls == 0);
    empty = false;
    { IfAddrsPtr result; assert(result.get_status() == 0 && result.get() == &owned); }
    assert(calls == 1);
}
"""

with tempfile.TemporaryDirectory() as directory:
    source = Path(directory) / "fixture.cpp"
    binary = Path(directory) / "fixture"
    source.write_text(fixture + wrapper[0] + main)
    subprocess.run(
        [os.environ.get("CXX", "c++"), "-std=c++17", "-Wall", "-Wextra", "-Werror",
         str(source), "-o", str(binary)],
        check=True,
    )
    subprocess.run([str(binary)], cwd=directory, check=True)
