# Hayro Phase 2 complete dependency compatibility matrix

Generated from the resolved Phase 1 host graph with native platform filtering.
Dev-only packages/edges excluded. Host means proc macros/build dependencies;
target+host packages are compiled separately for both domains. Features shown
come from cargo tree normal/build edges. H = unconditional std;
F = selected configuration requires std; A = alloc/core-compatible configuration.
A is source evidence, not blanket native execution certification. Build scripts
run on the host even for rows marked target. Source anchors include cfg/test code
and must be interpreted with the reviewed notes, never as mandatory API counts.

| Crate | Domain | Status | Resolved features | Finding | Source anchor |
| --- | --- | --- | --- | --- | --- |
| adler2 2.0.1 | target | A | — | Core/alloc utility | src/lib.rs:17, src/lib.rs:24, src/lib.rs:27 |
| alloc-no-stdlib 2.0.4 | target | A | — | Core/alloc utility | src/lib.rs:1 |
| alloc-stdlib 0.2.4 | target | H | — | Implicit std prelude; Box/Vec allocator adapter, no no_std feature | Cargo.toml / implicit prelude |
| arrayvec 0.7.8 | target | A | — | Core/alloc utility | src/lib.rs:23 |
| autocfg 1.5.1 | host | host std allowed | — | Host generator/derive tooling | src/lib.rs:63, src/lib.rs:64, src/lib.rs:65 |
| bitflags 2.13.2 | target | A | — | Core/alloc utility | src/lib.rs:272 |
| brotli 8.0.4 | target | F | alloc-stdlib, std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:1, src/lib.rs:17 |
| brotli-decompressor 5.0.3 | target | F | alloc-stdlib, std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:1, src/lib.rs:18, src/lib.rs:20 |
| bytemuck 1.25.2 | target | A | bytemuck_derive, derive, extern_crate_alloc, min_const_generics | Core/alloc utility | src/lib.rs:1, src/lib.rs:179 |
| bytemuck_derive 1.12.1 | host | host std allowed | — | Host generator/derive tooling | Cargo.toml / implicit prelude |
| byteorder-lite 0.1.0 | target | F | default, std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:27, src/lib.rs:71, src/lib.rs:3353 |
| cfg-if 1.0.5 | target | A | — | Core/alloc utility | src/lib.rs:32 |
| color 0.3.3 | target | F | bytemuck, std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:87 |
| crc32fast 1.5.2 | target | F | default, std | Core/alloc utility; selected std needs upstream feature adjustment; host build.rs | src/lib.rs:40 |
| euclid 0.22.14 | target | A | — | Source remains no_std; std forwarded to num-traits | src/lib.rs:10 |
| fastrand 2.5.0 | host | host std allowed | — | Host generator/derive tooling | src/lib.rs:101, src/lib.rs:115 |
| fdeflate 0.3.7 | target | H | — | No no_std flag; io Seek/Write in encoder | Cargo.toml / implicit prelude |
| fearless_simd 0.7.0 | target | F | std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:209, src/lib.rs:212, src/lib.rs:261 |
| fearless_simd 1.1.0 | target | F | default, std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:223, src/lib.rs:226, src/lib.rs:277 |
| flate2 1.1.10 | target | H | any_impl, any_zlib, default, miniz_oxide, runtime_detection, rust_backend, zlib-rs | Unconditional extern crate std despite #![no_std]; io Read/Write | src/lib.rs:124, src/lib.rs:131, src/lib.rs:140 |
| foldhash 0.2.0 | target | A | — | Defaults off; alloc/core hashing (metadata std belongs to inactive dev edges) | src/lib.rs:114 |
| font-types 0.12.6 | target | F | bytemuck, std | std optional; defaults off upstream | src/lib.rs:8, src/lib.rs:12 |
| guillotiere 0.7.0 | target | A | — | Core/alloc utility | src/lib.rs:2, src/lib.rs:12 |
| hashbrown 0.17.1 | target | A | default-hasher | Core/alloc utility | src/lib.rs:12, src/lib.rs:39 |
| hayro 0.8.0 | target | H | embed-cmaps, embed-fonts | No no_std switch; std Rc/RefCell; forces image PNG and Vello std/png | src/lib.rs:50, src/lib.rs:51, src/lib.rs:52 |
| hayro-ccitt 0.4.0 | target | A | — | Defaults off builds with native images probe | src/lib.rs:27 |
| hayro-cmap 0.1.0 | target | F | embed-cmaps, std | Defaults off is alloc-only; embed-cmaps forces std/LazyLock/Brotli | src/lib.rs:12 |
| hayro-interpret 0.8.0 | target | H | embed-cmaps, embed-fonts | No no_std switch; Mutex/OnceLock/LazyLock; forces syntax std/images/unsafe; moxcms | Cargo.toml / implicit prelude |
| hayro-jbig2 0.3.1 | target | F | fearless_simd, simd, std | Defaults off builds with native images probe; SIMD std enabled by syntax unsafe | src/lib.rs:14 |
| hayro-jpeg2000 0.4.1 | target | F | fearless_simd, simd, std | Defaults off builds with native images probe; SIMD std enabled by syntax unsafe | src/lib.rs:67 |
| hayro-postscript 0.1.0 | target | A | — | Alloc-only scanner; not a PostScript document renderer | src/lib.rs:17 |
| hayro-syntax 0.8.0 | target | F | images, std, unsafe | Defaults off builds natively; images also builds; unsafe implies std and flate2 | src/lib.rs:23, src/lib.rs:77 |
| image 0.25.10 | target | H | png | No no_std flag; io traits; Hayro selects png even for memory-only output | Cargo.toml / implicit prelude |
| kurbo 0.13.1 | target | F | default, std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:88 |
| linebender_resource_handle 0.1.1 | target | F | std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:21, src/lib.rs:25 |
| log 0.4.34 | target | A | — | Core/alloc utility | src/lib.rs:360, src/lib.rs:408, src/lib.rs:410 |
| lz4_flex 0.11.6 | target | A | — | Core/alloc utility | src/lib.rs:73 |
| memchr 2.8.3 | target | A | alloc | Core/alloc utility | src/lib.rs:173, src/lib.rs:198 |
| miniz_oxide 0.8.9 | target | A | default, simd, simd-adler32, with-alloc | Core/alloc utility | src/lib.rs:26 |
| miniz_oxide 0.9.1 | target | A | default, simd, simd-adler32, with-alloc | Core/alloc utility | src/lib.rs:26 |
| moxcms 0.8.1 | target | H | avx, avx_luts, avx_shaper_fixed_point_paths, avx_shaper_paths, default, in_place, lut, neon, neon_luts, neon_shaper_fixed_point_paths, neon_shaper_paths, sse, sse_luts, sse_shaper_fixed_point_paths, sse_shaper_paths | No no_std flag; std io Cursor/Read, time, SIMD dispatch and Arc | Cargo.toml / implicit prelude |
| num-traits 0.2.19 | target | F | default, std | Core/alloc utility; selected std needs upstream feature adjustment; host build.rs | src/lib.rs:19, src/lib.rs:23 |
| once_cell 1.21.4 | target | A | alloc, race | alloc/race only; no std feature selected | src/lib.rs:375 |
| peniko 0.6.1 | target | F | bytemuck, std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:25 |
| phf 0.13.1 | target | A | macros, phf_macros | Core/alloc utility | src/lib.rs:74, src/lib.rs:77 |
| phf_generator 0.13.1 | host | host std allowed | — | Host generator/derive tooling | src/lib.rs:6 |
| phf_macros 0.13.1 | host | host std allowed | — | Host generator/derive tooling | Cargo.toml / implicit prelude |
| phf_shared 0.13.1 | host+target | A | — | Core/alloc utility | src/lib.rs:6, src/lib.rs:9 |
| pic-scale 0.7.12 | target | H | — | No no_std flag even with defaults off; alloc/math; optional SIMD flags insufficient | Cargo.toml / implicit prelude |
| png 0.18.1 | target | H | — | No no_std flag; io traits and fdeflate/flate2 | Cargo.toml / implicit prelude |
| polycool 0.4.0 | target | F | std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:10 |
| proc-macro2 1.0.107 | host | host std allowed | default, proc-macro | Host generator/derive tooling; host build.rs | src/lib.rs:86, src/lib.rs:139, src/lib.rs:191 |
| pxfm 0.1.30 | target | H | — | No no_std flag; x86 OnceLock and AVX/FMA runtime detection | Cargo.toml / implicit prelude |
| quote 1.0.47 | host | host std allowed | default, proc-macro | Host generator/derive tooling; host build.rs | src/lib.rs:91, src/lib.rs:106 |
| read-fonts 0.43.3 | target | F | std | std optional; defaults off upstream | src/lib.rs:62, src/lib.rs:66 |
| rustc-hash 2.1.3 | target | F | default, std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:17, src/lib.rs:25, src/lib.rs:38 |
| simd-adler32 0.3.10 | target | F | const-generics, default, std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:79, src/lib.rs:219, src/lib.rs:267 |
| siphasher 1.0.4 | host+target | A | default, std | std feature is empty; source remains no_std; not a direct std blocker | src/lib.rs:2 |
| skrifa 0.46.2 | target | F | autohint_shaping, default, std | Disable defaults, choose libm upstream; embedded standard fonts avoid OS lookup | src/lib.rs:16 |
| smallvec 1.16.3 | target | A | const_generics, const_new | Core/alloc utility | src/lib.rs:100, src/lib.rs:111, src/lib.rs:126 |
| stable_deref_trait 1.2.1 | target | A | alloc | Core/alloc utility | src/lib.rs:17, src/lib.rs:149, src/lib.rs:151 |
| sun-img 0.2.4671-alpha.2 | target | A | — | Core/alloc utility | src/lib.rs:6 |
| sunlight-hayro-prototype 0.2.4671-alpha.2 | target | H | — | Host driver fs/Instant/System and unwind guard; intentionally host-only | src/lib.rs:4 |
| sunlight-ui 0.2.4671-alpha.2 | target | A | default | Core/alloc utility | lib.rs:27 |
| syn 2.0.119 | host | host std allowed | clone-impls, default, derive, full, parsing, printing, proc-macro | Host generator/derive tooling | src/lib.rs:251, src/lib.rs:315 |
| syn 3.0.8 | host | host std allowed | clone-impls, default, derive, extra-traits, fold, parsing, printing, proc-macro, visit | Host generator/derive tooling | src/lib.rs:366, src/lib.rs:430 |
| synstructure 0.14.0 | host | host std allowed | default, proc-macro | Host generator/derive tooling | src/lib.rs:164 |
| thiserror 2.0.21 | target | A | — | Core/alloc utility; host build.rs | src/lib.rs:262, src/lib.rs:277, src/lib.rs:279 |
| thiserror-impl 2.0.21 | host | host std allowed | — | Host generator/derive tooling | Cargo.toml / implicit prelude |
| unicode-ident 1.0.27 | host | host std allowed | — | Host generator/derive tooling | src/lib.rs:251 |
| unicode-segmentation 1.13.3 | target | A | — | Core/alloc utility | src/lib.rs:55, src/lib.rs:58 |
| vello_common 0.3.0 | target | F | png, std | libm alternative; png implies std; native SIMD codegen fails | src/lib.rs:53, src/lib.rs:61 |
| vello_cpu 0.3.0 | target | F | png, std, u8_pipeline | Defaults off + libm/u8_pipeline type-checks; native SIMD codegen fails | src/lib.rs:157, src/lib.rs:166 |
| yoke 0.8.3 | target | A | alloc, default, derive, zerofrom | Core/alloc utility | src/lib.rs:6 |
| yoke-derive 0.8.4 | host | host std allowed | — | Host generator/derive tooling | Cargo.toml / implicit prelude |
| zerofrom 0.1.8 | target | A | alloc, derive | Core/alloc utility | src/lib.rs:6 |
| zerofrom-derive 0.1.8 | host | host std allowed | — | Host generator/derive tooling | Cargo.toml / implicit prelude |
| zlib-rs 0.6.8 | target | F | rust-allocator, std | rust-allocator also disables no_std in this release; C allocator alternative exceeds scope | src/lib.rs:2 |
| zune-core 0.4.12 | target | A | — | Core/alloc utility | src/lib.rs:29 |
| zune-core 0.5.3 | target | F | std | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:46 |
| zune-inflate 0.2.54 | target | A | simd-adler32, zlib | Core/alloc utility | src/lib.rs:89 |
| zune-jpeg 0.4.21 | target | A | — | Core/alloc utility | src/lib.rs:102 |
| zune-jpeg 0.5.15 | target | F | neon, std, x86 | Core/alloc utility; selected std needs upstream feature adjustment | src/lib.rs:164 |
| zune-png 0.4.10 | target | A | — | Core/alloc utility | src/lib.rs:159 |

Total: 84 packages; 70 target participants; 14 host-only; 2 dual-domain.
