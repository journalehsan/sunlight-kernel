# Hayro native compatibility probe

Isolated Phase 2 experiment; not a PDF rendering backend. See
[results and reproduction](../../docs/hayro-native-phase2.md) and the
[dependency matrix](../../docs/hayro-phase2-matrix.md).

* Default: unmodified `hayro-syntax` in alloc-only mode.
* `images`: unmodified alloc-only PDF image decoders.
* `raster`: Vello libm/u8 pipeline + existing canvas pixel storage; original
  64x48 vector fixture, **not PDF interpretation**. Native type checking passes,
  code generation fails under the existing target configuration.
* `native-entry`: parser executable using the existing userspace allocator/CRT.
  Linked, not installed or boot-tested; 1 MiB heap; panic exits with status 2.
* `full-render`: deliberately reproduces the complete Hayro std blockers.

Only snapshot copy and canvas conversion reserve fallibly. Source/output limits
do not bound parser/rasterizer internal allocations for arbitrary PDFs. Internal
OOM is process failure, not a recoverable request error.

Do not enable all features when testing the alloc-only configuration: Cargo
feature unification makes `full-render` re-enable the std graph. The test harness
and diagnostic allocator use host std; the library stays no_std.
