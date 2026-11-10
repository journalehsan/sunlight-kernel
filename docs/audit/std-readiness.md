# ممیزی آمادگی std بومی — فاز ۱

## ۱. انجام‌شده و دامنهٔ شواهد

این گزارش بررسی منبع فعلی است، نه نتیجهٔ اجرای SunlightOS. PARTIAL یعنی مسیر اجرایی موجود است ولی قرارداد کامل CPython/C ABI یا رفتار guest تأیید نشده؛ STUB یعنی بدنه صریحاً unsupported است؛ MISSING فقط در دامنهٔ جست‌وجوی ثبت‌شده به معنی نبود API است؛ READY صرفاً برای قرارداد محدودِ منبع قابل استفاده است و هیچ ردیف زیر تأیید runtime نیست. نیازهای جدول همان خانواده‌های درخواست‌شده‌اند؛ فهرست کامل نیازهای configure نسخهٔ pinned CPython 3.12 برابر UNKNOWN است (docs/audit/python-port-audit.md:89).

مسیر std: `target/native-std/sysroot/lib/rustlib/src/rust/library/std`؛ overlay: `tools/native-std/overlay`؛ نحوهٔ انتخاب و کپی: tools/native-std/prepare.py:12، tools/native-std/prepare.py:40، tools/native-std/prepare.py:65. libc: sunlight-libc/src/lib.rs:1. target بومی و LLVM: tools/build_native_std.sh:10 و tools/targets/x86_64-sunlightos.json:9. target کرنل و host: .cargo/config.toml:5 و .cargo/config.toml:20؛ Helios/musl: tools/build.sh:127.

در جدول `O` یعنی `tools/native-std/overlay`، `L` یعنی `sunlight-libc/src` و `S` یعنی `target/native-std/sysroot/lib/rustlib/src/rust/library/std/src`. هر ارجاع با این پیشوندها مسیر واقعی فایل است.

| حوزه / تابع | وضعیت | پیاده‌سازی، محدودیت و نگاشت خطا | شاهد |
|---|---|---|---|
| file: open | PARTIAL | اعتبارسنجی flags/NUL با EINVAL؛ ABI اختصاصی، نه اثبات export کامل C | O/fs/sunlightos.rs:160؛ L/std_runtime.rs:153؛ L/lib.rs:102 |
| file: read | PARTIAL | syscall wrapper و تبدیل مقدار منفی به io::Error | O/fs/sunlightos.rs:190؛ L/lib.rs:170؛ O/pal/sunlightos/mod.rs:48 |
| file: write | PARTIAL | syscall wrapper؛ stderr از write_all استفاده می‌کند | O/fs/sunlightos.rs:193؛ L/std_runtime.rs:145 |
| file: lseek/tell | PARTIAL | whence و overflow در std؛ C wrapper مقدار -1 و errno می‌دهد | O/fs/sunlightos.rs:218؛ L/fd.rs:8 |
| file: fstat | PARTIAL | C wrapper موجود؛ struct اختصاصی و فاقد timestamps، نه struct stat استاندارد CPython | L/fd.rs:34؛ L/lib.rs:318؛ L/std_runtime.rs:172 |
| file: stat | PARTIAL | open(read)+fstat؛ stat مستقل از مجوز خواندن نیست | O/fs/sunlightos.rs:346 |
| file: stat timestamps | STUB | modified/accessed/created همگی unsupported؛ ctime/mtime/atime در Stat بومی موجود نیست | O/fs/sunlightos.rs:56؛ L/lib.rs:318 |
| file: truncate | STUB | unsupported؛ flagsِ open(truncate) مسیر جداگانه دارد | O/fs/sunlightos.rs:258؛ O/fs/sunlightos.rs:176 |
| file: fsync/datasync | STUB | unsupported، موفقیت صوری نیست | O/fs/sunlightos.rs:237 |
| file: rename | STUB | unsupported | O/fs/sunlightos.rs:301 |
| file: unlink | STUB | unsupported | O/fs/sunlightos.rs:297 |
| file: mkdir | STUB | unsupported | O/fs/sunlightos.rs:282 |
| file: readdir/lstat/canonicalize/copy | STUB | unsupported؛ مانع مسیرهای import عمومی | O/fs/sunlightos.rs:293؛ O/fs/sunlightos.rs:352؛ O/fs/sunlightos.rs:356؛ O/fs/sunlightos.rs:360 |
| file: locks/duplicate/permissions/set_times | STUB | unsupported؛ FileTimes setters محلی no-op هستند ولی set_times خطا می‌دهد | O/fs/sunlightos.rs:243؛ O/fs/sunlightos.rs:261؛ O/fs/sunlightos.rs:79؛ O/fs/sunlightos.rs:267 |
| file/terminal: flush | PARTIAL | Ok(())؛ std در این لایه بافر اختصاصی ندارد و write را مستقیم فراخوانی می‌کند؛ معادل fsync نیست، خرابی اثبات نشده | O/fs/sunlightos.rs:190؛ O/fs/sunlightos.rs:215؛ O/stdio/sunlightos.rs:26 |
| time: clock_gettime / Instant / SystemTime | PARTIAL | C ABI -1+EINVAL/EFAULT و خطای syscall؛ std در شکست panic می‌کند، نه موفقیت صوری | L/time.rs:94؛ L/time.rs:135؛ O/time/sunlightos.rs:52 |
| thread: spawn/join/detach/yield/sleep | PARTIAL | Rust threads موجود؛ spawn با errno، join خطای kernel را exit(101) می‌کند؛ sleep با deadline تکرار می‌شود | O/thread/sunlightos.rs:12؛ O/thread/sunlightos.rs:52؛ L/std_runtime.rs:250 |
| thread: pthread_create/join/keys C ABI | MISSING | در دامنه libc و overlay تطابق ندارد؛ Rust Thread جای pthread ABI نیست | docs/audit/std-source-review.log:1؛ O/thread/sunlightos.rs:12 |
| thread: set_name / parallelism / os_id | STUB | set_name no-op با خروجی unit؛ parallelism خطای UNKNOWN_THREAD_COUNT، id=None | O/thread/sunlightos.rs:45 |
| thread: TLS keys/destructors | PARTIAL | جدول ثابت 256 خانه، FS:0، ظرفیت تمام شود panic؛ آزمون guest وجود ندارد | O/thread_local/key/sunlightos.rs:6؛ O/thread_local/key/sunlightos.rs:15؛ O/thread_local/key/sunlightos.rs:36 |
| sync: mutex/rwlock/condvar/once/parking | PARTIAL | انتخاب پیاده‌سازی futex در std؛ اثبات ABI pthread نیست | tools/native-std/prepare.py:55؛ S/sys/sync/mutex/mod.rs:4؛ S/sys/sync/condvar/mod.rs:4؛ S/sys/sync/rwlock/mod.rs:4؛ S/sys/sync/once/mod.rs:13؛ S/sys/sync/thread_parking/mod.rs:4 |
| sync: futex wait/wake | PARTIAL | wait منفی panic؛ wake خطای منفی را false می‌کند؛ wake_all نتیجه را دور می‌اندازد، errno ندارد | O/pal/sunlightos/futex.rs:7؛ O/pal/sunlightos/futex.rs:20؛ L/std_runtime.rs:188 |
| signal: sigaction | MISSING | export libc در دامنه یافت نشد؛ kernel بومی STUB است و u64::MAX می‌دهد؛ این ENOSYS اختصاصی نیست | docs/audit/std-source-review.log:1؛ kernel/src/arch/x86_64/syscall.rs:10375؛ L/sys.rs:103؛ L/errno.rs:68 |
| signal: sigaltstack | MISSING | API بومی در دامنه libc/overlay پیدا نشد؛ Linux handler را نباید بومی شمرد | docs/audit/std-source-review.log:1؛ kernel/src/arch/x86_64/syscall.rs:1152 |
| process: std spawn/output | STUB | target خانواده unix ندارد؛ fallback unsupported؛ خطای Unsupported بدون errno عددی | tools/targets/x86_64-sunlightos.json:1؛ S/sys/process/mod.rs:18؛ S/sys/process/unsupported.rs:100 |
| process: std wait/try_wait | MISSING | Process(!) غیرقابل ساخت؛ بدنه wait=self.0، نه syscall یا موفقیت واقعی | S/sys/process/unsupported.rs:275 |
| process: native libc spawn/waitpid | PARTIAL | Rust API مستقل از std؛ waitpid Errno::Again را retry می‌کند | L/lib.rs:639؛ L/lib.rs:754 |
| net: TCP/UDP/DNS | STUB | انتخاب fallback؛ connect و lookup_host صریح unsupported؛ بقیه روش‌ها روی نوع غیرقابل ساخت‌اند | S/sys/net/connection/mod.rs:32؛ S/sys/net/connection/unsupported.rs:10؛ S/sys/net/connection/unsupported.rs:322 |
| net/sync: poll | PARTIAL | Rust wrapper؛ قرارداد مستند محدود به 1 تا 3 pipe readers و timeout=0/-1؛ C poll ABI تأیید نشده | L/lib.rs:557؛ L/lib.rs:567 |
| net: select/socketpair | MISSING | در دامنه libc/overlay تطابق ندارد؛ pipe معادل socketpair نیست | docs/audit/std-source-review.log:1؛ L/lib.rs:547 |
| env: get/set/unset/iterate | PARTIAL | std snapshot خصوصی تا 128 ورودی؛ set/unset موفق ولی libc getenv envp اولیه را می‌خواند: ناسازگاری قابل استنتاج از منبع، اجرای آن UNKNOWN | O/env/sunlightos.rs:4؛ O/env/sunlightos.rs:56؛ L/env.rs:35 |
| env: cwd/chdir/PATH/home/temp | PARTIAL | cwd با بافر 256؛ split/join و HOME/TMPDIR موجود؛ current_exe unsupported | O/paths/sunlightos.rs:4؛ O/paths/sunlightos.rs:17؛ O/paths/sunlightos.rs:58 |
| terminal: stdin/stdout/stderr | PARTIAL | read/write از ABI؛ flush=Ok، تست tty واقعی نیست | O/stdio/sunlightos.rs:21 |
| terminal: isatty | PARTIAL | fd=0/1/2 همیشه 1؛ سایر fdها EINVAL؛ بررسی واقعی tty/EBADF/ENOTTY ندارد، موفقیت احتمالی صوری | L/fd.rs:58 |
| terminal: tcgetattr | MISSING | export بومی در دامنه جست‌وجو یافت نشد؛ ioctl لینوکس اثبات بومی نیست | docs/audit/std-source-review.log:1 |

### نتیجهٔ بررسی error mapping

تمام STUBهای fs و process spawn و net connect از `unsupported()` خطا می‌دهند (O/pal/sunlightos/mod.rs:39). مقدار `UNSUPPORTED_PLATFORM` یک ErrorKind::Unsupported است، نه raw errno (S/io/error.rs:102). در C API آینده باید -1 همراه ENOSYS=38 بدهند (L/errno.rs:22)؛ تبدیل سادهٔ این خطای std به موفقیت یا تکیه بر raw_os_error معتبر نیست. نگاشت decode_error_kind عدد 38 را می‌شناسد ولی این به معنی تولید آن در تمام stubها نیست (O/io/error/sunlightos.rs:20).

خطای کلی native به Errno::Failed و سپس EIO نگاشت می‌شود؛ sigaction بومی فعلاً همین raw کلی را برمی‌گرداند، نه ENOSYS (L/sys.rs:103؛ L/errno.rs:68؛ kernel/src/arch/x86_64/syscall.rs:10377). `TooBig` در bridge به EINVAL کاهش می‌یابد و spawn به جز Again بقیه خطاها را ENOMEM می‌کند؛ دقت errno کامل نیست (L/std_runtime.rs:95؛ L/std_runtime.rs:220). موارد no-op/حذف نتیجه جداگانه در جدول آمده‌اند؛ از no-op بودن flush نتیجهٔ خرابی fsync گرفته نشده است.

## ۲. انجام‌نشده / مسدود

کد std/libc تغییر نکرده؛ build، ISO، دیسک و VM اجرا نشده‌اند. این گزارش فقط inventory منبع است. نبود C API در دامنهٔ محدود، نبود هر پیاده‌سازی در سراسر مخزن را اثبات نمی‌کند (docs/audit/std-source-review.log:1).

## ۳. UNKNOWN

تمام guest_runهای CSV با شمارش ثبت‌شده در docs/audit/std-source-review.log برابر UNKNOWN هستند؛ runner ممیزی در docs/audit/probes/README.md:46 موجود/پیکربندی‌شده اعلام نشده است. اجرای CPython M1/M2، export/link کامل C، سازگاری ABI و رفتار همزمانی guest برابر UNKNOWN باقی می‌مانند. نتایج Linux host نتیجهٔ SunlightOS نیستند (docs/audit/probes/README.md:28).

## ۴. gate و گام بعد

جدول inventory نوشته شد؛ gate مستنداتی فاز ۱ پاس است. گام بعد فقط تدوین برنامهٔ فاز ۲ است؛ اجرای تغییر و تست‌های پیشنهادی مجاز نشده است.
