# Compatibility Matrix

For public sample comparisons, host build targets and test results, see the [GDK sample audit](GDK_SAMPLE_AUDIT.md).

The retail matrix below is derived from the **actual imports** of `FortniteClient-XSX-Shipping.exe` (67 modules /
670 functions) and the exports of the Xbox system DLLs in `EmbeddedXvd/Windows/System32/`.
Raw per-DLL data: `analysis/fortnite_exe.json`, `analysis/syslib/*.json`.

## Evidence levels (CLAUDE.md §1)
1 Observed · 2 Documented · 3 Inferred · 4 Hypothesized · 5 Implemented · 6 Compiles ·
7 Unit-tested · 8 Integration-tested · 9 Verified vs target · 10 End-to-end.
The dependency inventory comes from the recorded imports and exports. The module notes identify runtime checks. On the progress map, the green GameInput and XAudio2 squares count the individual factory exports exercised by Coin Run on Proton. The four other listed XAudio2 imports retain their unverified status.

## Retail case study · module strategy

Legend — Plan: **BindOS** (forward to the real Windows DLL), **Reuse** (ship/forward to a
PC redistributable of the same component), **Reimpl** (write our own), **Stub** (minimal
no-op/offline impl to pass startup), **Translate** (reimpl that maps onto a host API).

| Module | Used by EXE | Role | Plan | Difficulty | Notes |
|---|---|---|---|---|---|
| `api-ms-win-core-*` (≈36) | many | Win32 API-Sets (kernel/sync/file/mem/thread…) | BindOS | low* | *per-symbol behaviour check vs desktop |
| `api-ms-win-crt-*` (≈12) | many | UCRT | BindOS | low | desktop UCRT |
| `ext-ms-win-rtcore-ntuser-window/message` | 9 | Windowing + message loop | BindOS→`user32` | low-med | map Xbox rtcore ntuser to real user32 |
| `msvcp140` / `vcruntime140(_1)` | — | MSVC C++ runtime | Reuse | none | shipped with the game |
| `bcrypt`, `crypt32` | few | Crypto | BindOS | low | desktop equivalents |
| `ws2_32` | ord | Winsock | BindOS | low | standard |
| `winhttp` | few | HTTP | BindOS | low | desktop equivalent |
| `mfplat`, `mfreadwrite` | few | Media Foundation (movies) | BindOS | low-med | desktop MF; not needed for first frame |
| `d3d12_xs.dll` | 6 | **Xbox D3D12** | **Translate**→PC `d3d12.dll` | **HIGH** | std COM interfaces + Xbox ext (`D3D12XboxCreateDevice`, `DXGIXPresentArray`, `D3DFlushEntireCpuCache`, `D3D12ConfigureVirtualMemoryX`) |
| `xg_xs.dll` | 1 | RDNA2 tiling/swizzle/DCC math | Reimpl / bypass | **MED-HIGH** | `XGCreateTextureComputer`; self-contained math, no HW dep |
| `xmem.dll` | 10 | Memory allocator | Reimpl→VirtualAlloc2/D3D12 heaps | MED | `XMemAlloc/Virtual/PhysicalPages`; thin over ntdll |
| `GameInput.dll` | 1 | Input (IGameInput) | Reuse PC GameInput / opt-in XInput adapter | MED | Homebrew on Proton verifies `GameInputCreate` ordinal 1, v0 polling, forward and backward history, device and reading callbacks, and two-motor `SetRumbleState` output on Linux virtual pads. Physical controllers and retail titles remain unverified. |
| `XAudio2_9.dll` | 5 (ord #1,2,4,5,6) | Audio | Reuse PC XAudio2_9 | MED | Coin Run verifies `XAudio2Create` (#1) and two simultaneous PCM voices with stereo panning via `SetOutputMatrix`, measured by Linux loopback on Proton. Reverb (#2), effects (#4), spatial audio (#5–6) and retail playback remain unverified. |
| `AcpHal.dll` | 6 | Xbox audio-DSP HAL | Stub/software | LOW-MED | `ApuAlloc`,`AcpHalCreate`,…; no PC ACP, route to software mix. `IAcpHal` is a null device: commands are validated and acknowledged in order, nothing is executed. SRC and DMA SHAPE contexts are allocated; EQ, filter/volume and PCM contexts are refused. |
| `xapu.dll` | 4 (FMOD GameCore build) | Audio-processor command queue | Refuse | LOW | `XApuConnect` fails with `E_NOTIMPL` after clearing its outputs; FMOD uses it only for hardware Opus decoding. Not imported by Fortnite. |
| `XCRDAPI.dll` | 1 | **DirectStorage** | Reuse PC DirectStorage / Reimpl file-backed | MED | `DStorageGetFactory`; for `.ucas` IoStore streaming |
| `dxcompiler_xs.dll` | 1 | Runtime DXIL compiler (DXC) | Reuse PC `dxcompiler.dll` | LOW-MED | `DxcCreateInstance` |
| `xgameruntime.dll` | **dynamic** | GDK services dispatcher | Reimpl | MED-HIGH | `QueryApiImpl` dispatch; loader must intercept its LoadLibrary |
| `EOSSDK-XboxOneGDK-Shipping.dll` | 48 | Epic Online Services | Stub (offline) | LOW→HIGH | 48 fns, mostly RTC/voice + Platform init; stub to pass startup |
| `GameChat2.dll` | delay | GDK chat | Partial | LOW | Coin Run loaded the desktop provider and round-tripped audio and network thread affinity. Fortnite chat remains untested. |

\* "low*" = the API-set *contract* is standard, but Xbox GameOS implementations may diverge
behind identical names; each must be validated, not assumed (CLAUDE.md principle; assumption A3).

## First-frame dependencies for the recorded retail build
1. **Loader** (map + DIR64 relocs + TLS + SEH + import resolution/redirection).
2. **BindOS** the ~52 standard modules; **Stub** EOS/GameChat/xgameruntime-nonessential.
3. **`xmem`** (memory) + GDK bootstrap enough for `FEngineLoop::PreInit`.
4. **`d3d12_xs`→PC D3D12 translation** + **`xg_xs`** + windowing for `RHIInit`/device.
5. First `DXGIXPresentArray`→ present.

## Retail GDK service surface (XGameRuntime)
Not exposed as named imports; reached via `xgameruntime.dll!QueryApiImpl` after a dynamic
load. The concrete XTaskQueue/XUser/XGameSave/etc. functions the title pulls will be
enumerated by (a) tracing `QueryApiImpl` call sites in the EXE using the symbol DB, or
(b) runtime instrumentation once the loader runs. **TBD — not yet enumerated.**

## Toolkit capabilities (verified, not game APIs)
| Capability | Level | Evidence |
|---|---|---|
| PE32+ parse (headers/sections/dirs/imports/exports/reloc/TLS) | 7 | tests; cross-checked vs `dumpbin`; parsed the 150 MB Fortnite EXE |
| ZIP64 inventory + extraction safety vetting | 7 | tests; vetted the real 25.5 GB archive (safe) |

## Known limitations
- The tested Fortnite build reaches the lobby with rendering defects. Retail gameplay remains unverified. [Retail test results](DEVELOPMENT_LOG.md)
- GDK (`QueryApiImpl`) service list not yet enumerated.
- Divergence of Xbox `api-ms-win-*` from desktop not yet validated per symbol.

## Scarlett DLL audit

All 16 supplied DLLs are x64 PE images. Their ordinary import tables contain
1,337 entries (611 distinct DLL/symbol pairs). The files have no delay-import
entries. The supplied sibling DLLs and the built TuxBox shims were checked by
export name or ordinal. A Windows probe resolved all 492 distinct host imports
across 61 modules with `LoadLibraryExA` and `GetProcAddress` under Proton
Experimental `experimental-11.0-20261027b`. This checks symbol availability.
Coin Run also exercised the local libraries and dispatched the embedded shaders separately. The progress map merges these imports with Fortnite’s by dependency DLL and exact symbol name or ordinal. [Reproduce the library checks](../tools/homebrew/README.md#local-scarlett-libraries) · [Audit snapshot](../tools/progress/scarlett-dll-imports.tsv) · [Progress map](img/progress.svg)

**All 611 distinct static imports resolve.** The standalone homebrew probe exercised 277 of the 492 host imports under Proton Experimental. It allocated an embedded-NUL string through OLE ordinal 4 and freed it through ordinal 6, verified CRT math and numeric parsing, exercised all 38 audited CRT stdio exports, checked saved-file contents and metadata, checked synchronization across threads and read a marshaled COM stream from a second thread. The report identifies the exact DLL, symbol and passing scenario. Retail caller behavior needs a separate title run. [Host-import results](../tools/progress/scarlett-import-checks.tsv)

**All 487 observed C/C++ runtime imports have behavior checks.** On Linux, 483 passed with Proton Experimental’s built-in UCRT and the supplied Scarlett C++ DLLs. Four passed with Microsoft’s optional x64 UCRT 10.0.26100.9457. On stock Proton, `_set_new_handler` and `_callnewh` failed the callback round trip, `__tzname` returned `PS` and `PD` for `PST8PDT`, and `_invoke_watson` reached an unimplemented export. Both providers passed the explicit `en-US` locale-name check. Together the host and runtime probes exercised 327/492 audited host imports with the stock UCRT and 330/492 across both providers. These results cover the recorded imports and tested scenarios. [Runtime evidence](../tools/progress/runtime-import-checks.tsv) · [C++/CX callback](../tools/progress/runtime-winrt-checks.tsv) · [Native UCRT evidence](../tools/progress/runtime-native-ucrt-checks.tsv)

Coin Run checks the BVH builder’s `xmem!XMemVirtualQuery` against reserved and committed process memory, including an invalid-handle case. The shim returns host memory state and protection, and reports zero for the console GPU-access fields. The upscaler’s `XMemGetWorkingSetStatistics` still returns `E_NOTIMPL`. Coin Run uses `GetModuleHandleExW` to recover the owning module from an export address and `GetModuleFileNameW` to check that module’s on-disk path. The Scarlett atomic wait DLL calls the host’s `WaitOnAddress` and `WakeByAddressSingle` during the timeout, unequal-value return and cross-thread notification checks.

| Supplied DLL | Import entries | Exports | Translation evidence and open work |
|---|---:|---:|---|
| `amd_fidelityfx_loader_xbox.dll` | 22 | 5 | Four render-resolution queries returned the expected sizes through the loader. The separate frame-generation provider was absent. |
| `amd_fidelityfx_upscaler_xbox.dll` | 86 | 5 | The same four queries passed directly on the provider. Seventeen of 29 embedded kernels pass GPU readback, including sharpening, 12-level exposure reduction and a 16-to-32-channel convolution. Twelve kernels and full-frame upscaling remain open. The model-specific DLL name is the provider’s own PE export name. |
| `XboxBvhBuilder.dll` | 57 | 12 | Eighteen of 52 embedded kernels pass GPU readback, including two strided node copies. Four of them also complete a stable radix sort of 16,384 key/index pairs. The native DLL loads and its validator export resolves. A complete acceleration-structure build remains unverified. |
| `Bink2Scarlett.dll` | 60 | 74 | Coin Run set and retrieved a Bink error string. The host resolves the XAudio2 ordinal. Synchronized audio and video playback remains unverified. |
| `libHttpClient.GDK.dll` | 198 | 129 | The version query and offline HTTP request setup passed under Proton. The startup probe initializes the public desktop Gaming Runtime before calling `HCInitialize`. Network transfers and WebSocket sessions remain unverified. |
| `XCurl.dll` | 140 | 61 | URL decoding returned three bytes including an embedded NUL, and the DLL freed the result. Strings name `xgameruntime.dll`, `gameplatformservices.dll`, `QueryApiImpl` and a URL-security query. Network transfers and these dynamic lookups remain unverified. |
| `concrt140.dll` | 103 | 291 | The native concurrency runtime reported an available processor count within the host system limit. |
| `msvcp140.dll` | 187 | 1493 | All 194 observed exports passed, including narrow and wide streams, locale facets, synchronization and exception propagation. The binary also names `appmodel.dll`. |
| `msvcp140_1.dll` | 33 | 7 | The initial aligned default memory resource matched the aligned new/delete resource. |
| `msvcp140_2.dll` | 96 | 44 | The native beta function returned 1/12 for inputs 2 and 3. |
| `msvcp140_atomic_wait.dll` | 64 | 32 | Zero-initialized allocation, immediate timeout, unequal-value return and waking one waiting thread passed. The binary also names `icu.dll`. |
| `msvcp140_codecvt_ids.dll` | 24 | 4 | The two exported character-conversion facet IDs resolved to distinct addresses. |
| `vccorlib140.dll` | 114 | 256 | The C++/CX fixture caught the preallocated platform exception returned by the registered out-of-memory callback. WinRT activation remains unverified. The binary also names `gcombase.dll`. |
| `vcomp140.dll` | 78 | 113 | The serial OpenMP team reported one thread with index zero. The binary also names `vcomp120ui.dll`. |
| `vcruntime140.dll` | 41 | 71 | Memory operations, runtime type identity, nonlocal jumps and typed exception unwinding passed. |
| `vcruntime140_1.dll` | 34 | 3 | The supplied ostream caught a throwing homebrew stream buffer and set badbit. Its x64 unwind entry selected the supplied `__CxxFrameHandler4`. |

Only the upscaler and BVH builder contain identifiable DXBC shader containers.
The ten Microsoft runtime DLLs report version 14.29.30157.0. Their static imports
are satisfied by the supplied runtimes and host APIs. DLL and function names in
strings identify candidates for dynamic lookup. Whether an application takes
those paths, uses a fallback or requires a particular GDK service depends on
the workload. [GPU readback procedures](../tools/homebrew/README.md#retail-fidelityfx-prepass)

Three BVH kernels account for six unresolved VOPC squares. The geometry kernel at DLL offset `0x8a5e0` uses `v_cmp_gt_u64` and `v_cmp_le_u64` while following two GPU addresses supplied in root constants. TuxBox needs to resolve those addresses to buffer ranges at dispatch. The instance kernels at `0x5f040` and `0x72f20` also need masked wave32 min/max reductions and floating-point atomics on indirect addresses. Between them they contain `v_cmp_eq_u64`, `v_cmp_ne_u64`, `v_cmp_eq_u16` and `v_cmpx_eq_u16`. [BVH instruction audit](../tools/progress/bvh-mnemonics.txt)

### Package startup evidence

For the inspected Scarlett package version 1.0.2.1, both manifests name `game_scarlett_master.exe` at the package root. The install layout lists 104 file entries across 25 chunks. The executable in chunk 1001 is the sole absent entry. All 16 libraries in chunk 1002 match the audit by dependency and exact import name or ordinal. This inventory checks file presence. The layout provides no content hashes for the listed assets.

The homebrew startup probe ran on Proton Experimental with Microsoft.GDK.Windows 2610.0.9661. The desktop Gaming Runtime accepted the package’s `MicrosoftGame.Config` and returned the declared title ID through `XGameGetXboxTitleId`. The staged Scarlett HTTP client then initialized, created a request, returned the expected method and URL, and released the handle. The same request passed with default runtime initialization. Calling `HCInitialize` in the original 16-DLL environment returned `E_GAMERUNTIME_DLL_NOT_FOUND`. The package declares multiple-user support, advanced Series X and S memory profiles, SMT, and 2,048 MB of persistent local storage. Memory profiles, SMT and storage still need runtime tests. The existing Fortnite-specific user and store integration is gated on its executable build. [Reproduce the startup checks](../tools/homebrew/README.md#scarlett-package-startup) · [Gaming Runtime error codes](https://learn.microsoft.com/en-us/xbox/gdk/docs/reference/errorcodes)

With that same config, the desktop runtime reported a maximum of one user for this session. User-change registration and unregistration both succeeded. A silent default-user request reached its completion callback and returned `E_GAMEUSER_NO_DEFAULT_USER` with no handle. The probe observed zero user-change notifications. Signed-in identity lookup and delivery of a real account transition still need a session with a user. [GDK silent user selection](https://learn.microsoft.com/en-us/gaming/gdk/docs/reference/system/xuser/enums/xuseraddoptions)

### Scarlett package assets

The asset section of the progress map covers package 1.0.2.1. Each square names a specific check. The audit and probe produced the following results. [Measured counts](../tools/progress/scarlett-assets.json) · [Reproduce the checks](../tools/homebrew/README.md#scarlett-package-assets)

| Asset | Verified locally | Open question |
|---|---|---|
| RPF9 | All 22 archives have the RPF9 signature. A Windows homebrew process on Proton matched 52 host byte ranges across those archives, including reads spanning 4 GiB in eight files. | Can the title resolve an archive member by name and stream its contents? |
| TTPF | All 57 stores passed RDS5 chunk-boundary and ETPF descriptor-table checks. They contain 357,115 descriptors with 20 distinct DXGI format IDs. The virtual stream exceeds 4 GiB in 18 stores. Proton matched 114 physical byte ranges. Coin Run sampled two single-mip, mode-0 RGBA8 textures from the graphics store. All 131,072 texels matched independent 6Stars reference hashes. | Which layouts and storage APIs does the title use? |
| Texture compression | One complete zlib chunk from each store inflated to its indexed virtual size and passed its checksum. The audit records all observed codec tags. | Can a BCPack payload be decoded against known-good full-resolution output? |
| Shader cache | A checksummed zlib member in `shaders.rpf` contains a `PS 00 09` header declaring 25,471 graphics PSOs, 2,693 compute PSOs, 54,576 metadata records and 21,522 shader binaries. Homebrew draws and dispatches verify specific original shader records against independent pixel, depth and buffer references. The snapshot lists each PSO, shader hash and readback count. | Can the title bind the original pipeline and render with its own resources? |

6Stars documents the texture descriptor layout, chunk codec tags and shader-cache fields. Its format-identification and preview results are useful references for fixture selection. TuxBox tracks its own measurements. [6Stars texture format coverage](https://github.com/Ahmet-Dedeler/6Stars/blob/eb65e94e0169b5bedf294466c1b9bf2561cb7591/docs/format-coverage.md) · [6Stars PSO cache parser](https://github.com/Ahmet-Dedeler/6Stars/blob/eb65e94e0169b5bedf294466c1b9bf2561cb7591/src/rdna2.rs)

The VOPC inventory covers all 21,522 indexed shader records, including a 52-byte vertex-fetch subroutine. It found 69 distinct comparison instructions. Each tooltip gives the number of cache shaders containing that instruction and the number with verified GPU outputs. For an instruction seen only in this cache, green requires successful readback from every containing program. The full Fortnite and middleware translation audits retain their own denominators. All 190 named GFX10.3 comparisons also passed a separate test against native Radeon hardware. Translated VCC and EXEC bits matched the native results for 243,240 vectors across both machine encodings and five encoding/mode combinations. Both the NVIDIA and Radeon Proton renderers matched these results. A further 108,248 vectors exercised 182 subword addressing (SDWA) encodings, including byte and half-word selection before floating-point classification. [Native differential results](../tools/progress/native-vopc.tsv) · [VOPC inventory](../tools/progress/scarlett-vopc-candidates.tsv) · [AMD RDNA2 ISA, section 12.9](https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf)

The engine may parse these containers itself. Once the executable is available, rerun preflight for its ordinary and delay-loaded imports, then trace the first archive open and completed read under Proton. Record the requested path, API, offset, byte count and result. That will show whether the next compatibility change belongs in file access, a DirectStorage queue, decompression or graphics.

### Scarlett command-list UAV clears

In the inspected FidelityFX upscaler (SHA-256 `faff02f294a7e9149ce079dffe3dedbcabe99b1f180ab14222aa8689e00a75fa`),
the clear job binds the descriptor heap and flushes pending barriers. Both branches
pass GPU and CPU descriptor handles, the resource, four clear components, a zero
rectangle count and a null rectangle pointer. The method names are inferred from
these calls and AMD’s public DX12 clear implementations. [AMD integer clear](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/c6efa6bf7f2027b3ec94f28578bb5965eabb9e55/sdk/src/backends/dx12/ffx_dx12.cpp#L3421-L3426) · [AMD float clear](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/d7531ae47d8b36a5d4025663e731a47a38be882f/sdk/src/backends/dx12/ffx_dx12.cpp#L1996-L2004)

| Xbox slot | Call RVA | Clear values at the call | PC method |
|---|---|---|---|
| 53 | `0x72ee` | Four color words copied into a temporary array | `ClearUnorderedAccessViewUint` (49) |
| 54 | `0x7296` | Pointer to the job’s original float color | `ClearUnorderedAccessViewFloat` (50) |

The `--uav-test` homebrew fixture runs independently of the DLL. It checks all
54 texels of each 9×6 surface after a full clear followed by a 5×3 rectangular
clear. The integer case preserves the RGBA byte values. The float case checks
conversion to RGBA8 UNORM. [Run the fixture](../tools/homebrew/README.md#graphical-coin-run)

### Scarlett resource discard

At RVA `0x6dcb` the same FidelityFX DLL takes a resource from its resource table,
flushes pending barriers and calls command-list slot 55 with the resource and a
null region. This matches AMD’s discard job. TuxBox forwards the call to PC
`DiscardResource` (slot 51). [AMD discard job](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK/blob/c6efa6bf7f2027b3ec94f28578bb5965eabb9e55/sdk/src/backends/dx12/ffx_dx12.cpp#L3431-L3444)

A dispatch-table regression test checks the selected host method. On Proton,
`--discard-test` exercises the observed null-region call and a synthetic
rectangle using the Windows SDK region layout. It overwrites discarded pixels
before readback and checks that surrounding pixels retain their color.
[Discard semantics](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12graphicscommandlist-discardresource)

### Committed textures, copies and indirect commands across GXDK editions

Microsoft publishes a Scarlett upscaler DLL (SHA-256 `3ffe4c8646ea3ac5f15ef6df49a019311bb5ba0ae9d491e6cc957d2b62584880`) with the Xbox samples. Its `.xbld` section identifies April 2026 Update 2 (260402). The supplied upscaler above identifies October 2026 (261000). The following RVAs are relative to the image base. [Public sample binary](https://github.com/microsoft/Xbox-GDK-Samples/blob/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Kits/FidelityFX/signedbin/amd_fidelityfx_upscaler_xbox.dll)

| Interface and slot | April RVA | October RVA | Observed behavior and translation |
|---|---|---|---|
| Device 76 | `0x3883` | `0x33cc` | `CreateCommittedResourceX` supplies a resource description with ordinary fields through byte 51 and DCC options beginning at byte 52. TuxBox preserves the logical resource and lets the PC driver choose compression and the layout for private GPU textures using Xbox layout `0x101`. The April binary also uses flags `0xc00005` for a render target with UAV access. |
| Command list 85 | `0x6dff` | `0x7147`, `0x7191` | Both texture-copy branches use the desktop location layout and a trailing zero. TuxBox forwards coordinates, source box and locations to PC `CopyTextureRegion`. Other trailing flag values are unverified. |
| Command list 61 | `0x6a71` | `0x67ac` | `ExecuteIndirect` passes a command signature, maximum count of one, argument buffer and job offset, then a null count buffer and zero count-buffer offset. |
| Command list 59 | `0x2b9f`, `0x79b2` | `0x6b5a`, `0x6e52` | PIX packets carry an event kind, data pointer and byte size. The host receives the PIX v2 metadata tag. The earlier RHI uses this slot for `ExecuteIndirect`. |
| Command list 60 | `0x2b7b`, `0x7948` | — | The timing path provides an event kind without a packet. TuxBox preserves host GPU event boundaries with a generic label. Xbox CPU/GPU event correlation remains open. |
| Device 67 | `0x5765` | `0x56d9` | The offline compute pipeline path requests `ID3D12PipelineState`. The October binary explicitly logs a fallback on failure and retries at slot 62 (RVA `0x5825`). TuxBox clears the output and returns `E_NOTIMPL` so the caller can create an ordinary compute pipeline. Offline packet deserialization remains open. |

The GDK sample source provides the named calls and resource fields. Microsoft’s PIX headers describe how the packet and timing paths select begin, end and marker operations. TuxBox uses the edition of the module issuing the call to distinguish slots 59–61. For unmarked modules and older unrecognized editions it retains the original slot-59 indirect path. Conflicting tags or an unreadable edition section select a diagnostic probe. Only the April timing path has a verified slot-60 call. [Resource creation](https://github.com/microsoft/Xbox-GDK-Samples/blob/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Kits/AMDTK/fidelityfx/src/backends/gdk/ffx_gdk.cpp#L1398-L1431) · [Indirect dispatch and copies](https://github.com/microsoft/Xbox-GDK-Samples/blob/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Kits/AMDTK/fidelityfx/src/backends/gdk/ffx_gdk.cpp#L2988-L3059) · [PIX event encoding](https://github.com/microsoft/PixEvents/blob/b0caa735f8510f4ff60c29ef1c88101620defc5e/include/PIXEvents.h#L406-L446) · [PIX desktop forwarding](https://github.com/microsoft/PixEvents/blob/b0caa735f8510f4ff60c29ef1c88101620defc5e/include/pix3_win.h#L452-L523)

The homebrew copy, indirect and compute-fallback tests use original shaders and synthetic inputs. The two tagged fixture DLLs contain just the sample edition string. The tests check GPU output under Proton, including the zero-count indirect path. [Run the tests](../tools/homebrew/README.md#graphical-coin-run)

The optional public-binary test loads the pinned Xbox loader and upscaler with Microsoft's x64 WinPixEventRuntime 1.0.240308001 supplying the PIX import. Four render-resolution queries pass under Proton. Creating an FSR 3.1.6 context exercises the private texture layout (`0x101`, assigned at April RVA `0x3732`) and reaches device slot 67. Its first offline compute description contains a root signature followed by four pointer/size pairs. The observed lengths in bytes are 28 for the packet, 20 for packet metadata, 7,296 for GPU instructions and 504 for shader metadata. This April provider returns a runtime error with a null context after deserialization fails. The default FSR 4 path separately requests a UAV texture with format `R9G9B9E5_SHAREDEXP`, which the tested host rejects. Upscaled output remains unverified. [Public sample setup](https://github.com/microsoft/Xbox-GDK-Samples/blob/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Samples/Graphics/AMD_FSR/readme.md) · [Offline pipeline components](https://github.com/microsoft/Xbox-GDK-Samples/blob/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Samples/Graphics/HlslCompile/AcoPipelineState.cpp#L54-L86)

### GXDK 250405 compute UAV barrier

The supplied BVH builder calls command-list slot 146 at RVAs `0xd7b4` and `0xd80a`. Both pass one global barrier group containing one barrier. The four 32-bit fields at RVA `0xcdf60` specify compute shading (`0x80`) before and after the barrier and unordered access (`0x10`) on both sides. TuxBox recognizes that form for callers tagged GXDK 250405 and records a desktop UAV barrier with a null resource. This orders UAV accesses across resources on the command list and works with the base D3D12 interface. Other barrier groups and access masks are unverified. [Enhanced barrier layout](https://microsoft.github.io/DirectX-Specs/d3d/D3D12EnhancedBarriers.html#d3d12_global_barrier) · [Desktop UAV ordering](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-d3d12_resource_uav_barrier)

The homebrew test records two dependent compute dispatches, checks the host barrier trace and reads back 32 incremented values. The tracker marks slot 146 partial because the translation handles this single global compute UAV barrier form. [Run the fixture](../tools/homebrew/README.md#graphical-coin-run)

### GXDK 250405 thread-count dispatch

At RVAs `0x14972` and `0x15215`, the BVH builder writes a five-word inline dispatch packet. When the packet buffer is full, it passes the same X, Y, Z and control word to command-list slot 80. The observed values `0x20` and `0x8020` match the RDNA2 thread-dimension and wave32 control bits. TuxBox interprets these calls as thread counts and divides each axis by the bound shader’s group dimension before issuing desktop `Dispatch`. [AMD register definitions](https://github.com/torvalds/linux/blob/master/drivers/gpu/drm/amd/include/asic_reg/gc/gc_10_3_0_sh_mask.h) · [Desktop dispatch dimensions](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12graphicscommandlist-dispatch)

The translation covers whole groups for a subset of straight-line shaders using ordinary register operands, buffer loads and stores, and lane-local arithmetic. Wave-sensitive instructions, unrecognized shaders and control words, partial groups and counts above the desktop limit are skipped with a diagnostic. The homebrew test verifies both observed control words, pipeline switching and the bounds of GPU writes. Eighteen BVH builder shaders have passed readback through ordinary group dispatch. The library’s own slot 80 dispatch sequence remains unverified. [Run the fixture](../tools/homebrew/README.md#graphical-coin-run)

### Open GPU synchronization calls

The supplied `XboxBvhBuilder.dll` (GXDK 250405, SHA-256 `99f803ac22c01ef329dddb1d40be67238d2a812d01e8d040b3cb5135bc23707e`) uses a table of four-byte counters. At RVAs `0xd679` and `0xd6de`, command-list slots 73 and 71 receive the same counter address. Slot 73 receives a comparison value and a 32-bit mask. Slot 71 receives an incremented value and either zero or `0x1000`. Slot 90 receives a counter address and zero at RVA `0xe836`. Microsoft's pipelined postprocessing sample demonstrates GPU-side waits and bottom-of-pipe writes with *64-bit* counters. The 32-bit flags and ordering between queues remain open. [Microsoft synchronization sample](https://github.com/microsoft/Xbox-GDK-Samples/blob/b1e77b6ac10f95b07296025fda171ca2ddc42dbe/Samples/Graphics/PipelinedPostprocess/PipelinedPostprocess.cpp#L278-L300)

The exported `BuildRaytracingAccelerationStructure` at RVA `0x21610` copies the 48-byte build description into a batch entry. At `0xd069` the builder copies the field at byte 40 into the job. That field matches `ScratchAccelerationStructureData`, a GPU virtual address. Slots 71, 73 and 90 address four-byte words within this scratch buffer. TuxBox translates the non-final zero write from slot 90 when the last explicit barrier establishes an aligned destination in a `COPY_DEST` buffer and the host supports immediate writes. Coin Run initialized three adjacent words to `[13, 29, 71]` and read back `[13, 0, 71]` on NVIDIA and AMD. The first word exercised the final-batch flag and the last exercised a buffer transitioned to `COPY_SOURCE`. Both cases are logged and skipped. [Build description](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-d3d12_build_raytracing_acceleration_structure_desc) · [Desktop immediate-write requirements](https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12graphicscommandlist2-writebufferimmediate)

### D3D12.x slot tracking

The map covers device slots 0–159 and command-list slots 0–148. Each interface keeps its original baseline and grows through the highest recorded slot, including the intervening positions. The map distinguishes diagnostic bindings from translated behavior in each slot’s tooltip. [Tracker data](../tools/progress/progress.json)

Device has 56 implemented or forwarded slots and 104 partial slots. For each slot from 91 through 159, Coin Run verified that the wrapper logs the slot, caller module and return-address RVA, then returns `E_NOTIMPL`. A tagged fixture DLL independently called slot 159 and appeared in its trace. These 69 diagnostic bindings can identify future retail callers. The method names, signatures and behavior are still unknown. [Run the Device probe](../tools/homebrew/README.md#graphical-coin-run)

CommandList has 57 implemented or forwarded slots and 92 partial slots. Of the partials, 88 have diagnostic bindings only. Coin Run calls those slots plus slot 90 through the untagged guest vtable, verifies the zero return and caller location, then checks GPU readback. A tagged DLL independently exercises slot 148. The following retail calls have additional evidence. [Run the CommandList probe](../tools/homebrew/README.md#graphical-coin-run)

| Command-list slot | Caller and call RVA | Evidence | Status |
|---|---|---|---|
| 71, 73 | BVH builder 250405 · `0xd6de`, `0xd679` | Both calls address the same scratch counter. Their ordering and flag semantics remain open. | Diagnostic only |
| 80 | BVH builder 250405 · `0x149ba`, `0x1525d` | Thread-count dispatch translates whole groups for the verified shader subset. | Partial |
| 90 | BVH builder 250405 · `0xe836` | The non-final 32-bit counter clear passes GPU readback in an explicitly transitioned buffer. | Partial |
| 141 | FidelityFX 260402 · `0x68a5`, 261000 · `0x65f8` | Both callers pass the GPU address and size of the buffer they name `FFX_XBox_ComputeScratch`. The name is at RVA `0x67478` in April and `0x482b0` in October. | Diagnostic only |
| 146 | BVH builder 250405 · `0xd7b4`, `0xd80a` | The single global compute UAV barrier form passes GPU readback under Proton. | Partial |
| 148 | BVH builder 250405 · `0x114af`, `0x14afb` | Callers pass a byte size and 8 or 16. The second caller writes records through the returned pointer. | Diagnostic only |
