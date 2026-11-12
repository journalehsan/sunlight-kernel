#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash tools/build_native_c.sh
mkdir -p target/native-cxx
python3 tools/native-c/cc.py -std=c++17 -Wall -Wextra -Werror -I sunlight-cxx/include -c sunlight-cxx/src/ostream.cpp -o target/native-cxx/ostream.o
python3 tools/native-c/cc.py -std=c++17 -Wall -Wextra -Werror -I sunlight-cxx/include -c sunlight-cxx/src/string.cpp -o target/native-cxx/string.o
python3 tools/native-c/cc.py -std=c++17 -Wall -Wextra -Werror -I sunlight-cxx/include -c sunlight-cxx/src/exception.cpp -o target/native-cxx/exception.o
abi=sunlight-cxx/vendor/libcxxabi
for source in private_typeinfo stdlib_typeinfo; do
    python3 tools/native-c/cc.py -std=c++17 -fno-exceptions -I sunlight-cxx/include -I sunlight-cxx/abi-include -I "$abi/include" -c "$abi/src/$source.cpp" -o "target/native-cxx/$source.o"
done
python3 tools/native-c/cc.py -std=c++17 -O2 -fno-exceptions -D_LIBCXXABI_USE_FUTEX -I sunlight-cxx/include -I sunlight-cxx/abi-include -I "$abi/include" -c "$abi/src/cxa_guard.cpp" -o target/native-cxx/cxa_guard.o
python3 tools/native-c/cc.py -std=c++17 -O2 -fno-exceptions -I sunlight-cxx/include -I sunlight-cxx/abi-include -c sunlight-cxx/src/guard_support.cpp -o target/native-cxx/guard_support.o
python3 tools/native-c/cc.py -std=c++17 -Wall -Wextra -Werror -I sunlight-cxx/include -c sunlight-cxx/src/abi_support.cpp -o target/native-cxx/abi_support.o
rm -f target/native-c/sysroot/lib/libsunlight_cxx.a
llvm-ar rcs target/native-c/sysroot/lib/libsunlight_cxx.a target/native-cxx/ostream.o target/native-cxx/string.o target/native-cxx/exception.o target/native-cxx/private_typeinfo.o target/native-cxx/stdlib_typeinfo.o target/native-cxx/cxa_guard.o target/native-cxx/guard_support.o target/native-cxx/abi_support.o
ln -sfn "$PWD/sunlight-cxx/include" target/native-c/sysroot/include-c++
unwind=sunlight-cxx/vendor/libunwind
for source in cxa_exception cxa_personality; do
    python3 tools/native-c/cc.py -std=c++17 -O2 -I sunlight-cxx/include -I sunlight-cxx/abi-include -I "$abi/include" -I "$unwind/include" -include sunlight-cxx/abi-include/native_unwind.h -c "$abi/src/$source.cpp" -o "target/native-cxx/$source.o"
done
for source in eh_support entry new lifecycle; do
    python3 tools/native-c/cc.py -std=c++17 -O2 -I sunlight-cxx/include -I "$abi/include" -I "$abi/src" -I "$unwind/include" -c "sunlight-cxx/src/$source.cpp" -o "target/native-cxx/$source.o"
done
for source in libunwind.cpp UnwindLevel1.c UnwindLevel1-gcc-ext.c UnwindRegistersRestore.S UnwindRegistersSave.S; do
    python3 tools/native-c/cc.py -O2 -fexceptions -DNDEBUG -D_LIBUNWIND_IS_BAREMETAL -D_LIBUNWIND_NO_HEAP -I "$unwind/include" -I sunlight-cxx/abi-include -include sunlight-cxx/abi-include/native_unwind.h -c "$unwind/src/$source" -o "target/native-cxx/$source.o"
done
rm -f target/native-c/sysroot/lib/libsunlight_eh.a
llvm-ar rcs target/native-c/sysroot/lib/libsunlight_eh.a target/native-cxx/cxa_exception.o target/native-cxx/cxa_personality.o target/native-cxx/eh_support.o target/native-cxx/entry.o target/native-cxx/new.o target/native-cxx/lifecycle.o target/native-cxx/libunwind.cpp.o target/native-cxx/UnwindLevel1.c.o target/native-cxx/UnwindLevel1-gcc-ext.c.o target/native-cxx/UnwindRegistersRestore.S.o target/native-cxx/UnwindRegistersSave.S.o
