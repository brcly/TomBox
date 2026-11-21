# Technical Debt / Known Gaps / Unverified Assumptions

## Current platform gaps

- Proton and Windows load prepared PE images and the app-local DLLs. Retail startup on Proton still needs a title run. [Execution model](ARCHITECTURE.md)
- Graphics, virtual gamepads and PCM audio have homebrew tests on Proton. XMA2 playback, full-frame FidelityFX upscaling, and physical controller coverage remain open. [GDK sample audit](GDK_SAMPLE_AUDIT.md)
- DirectStorage’s file-to-memory path checks capacity for raw and zlib reads. The shim logs failures. Queue error records, cancellation, GPU destinations and additional codecs need separate implementation and tests.
- Proton Experimental reported an unimplemented Cabinet export when running the desktop build of Microsoft’s `xbcompress`. [Reproduction](GDK_SAMPLE_AUDIT.md#reproduce-the-desktop-comparison-on-linux)

## Independent validation done
- `pe-inspect` cross-checked against **Microsoft `dumpbin`** on the same binary:
  machine/sections/magic/entry-point/image-base/subsystem and the full import
  module list all agree. (CLAUDE.md §3A — compare against independent tools.)

## Parser test backlog from the initial Windows investigation
- Base-relocation parser: a fixture asserting exact block/entry/type counts
  (currently only exercised, not value-asserted).
- TLS parser: a fixture asserting callback count on a binary with TLS callbacks.
- A hand-built minimal PE32+ fixture for fully deterministic unit tests that do not
  depend on `C:\Windows\System32`.
- `cargo fuzz` / sanitizer builds not yet configured (MSVC target; consider a
  property-test loop as an interim — the random/truncation tests partially cover this).

## Initial retail investigation (historical hypotheses)
- **A1** The extracted EXE is a readable PE32+, not an encrypted blob. _[premise from user; verify first]_
- **A2** Game code is native x64 needing no instruction translation. _[strong inference; confirm machine + .pdata]_
- **A3** Imports reference console D3D12.x + GDK modules distinct from PC DLLs. _[expected; confirm]_
- **A4** Host Zen 3 ISA is a superset of whatever the binary needs. _[likely; confirm via instruction scan]_
- **A5** Fortnite = Unreal Engine; UE narrows the GDK surface at startup. _[expected; confirm engine version]_

## Risks recorded at the start of the retail investigation
- **R1 (high)** Shader format: if shaders ship as Xbox microcode rather than DXIL,
  the graphics path is dramatically harder. Unknown until the shader container is found.
- **R2 (high)** D3D12.x semantic differences from PC D3D12 are NDA-gated; must be
  reverse-engineered from usage. A shader translator alone is not enough.
- **R3 (med)** GameOS/NT API behavior differences hidden behind familiar names.
- **R4 (med)** No MSVC-CMake/clang on PATH; C++ interop (e.g. with host D3D12 headers)
  will go through Rust's `windows` crate or a located MSVC toolchain, not system CMake.
- **R5 (med)** Anti-cheat / online dependencies at startup could block a pure offline
  boot; must map whether first-frame requires any network/service call (we will not
  connect to production services).
- **R6 (low/now)** 25.5 GB input: disk and iteration cost; keep derived analysis small.

## Original Windows test machine
- No real Python interpreter (Store alias stub only).
- `cmake`/`ninja`/standalone `clang`/`dumpbin` not on PATH (MSVC toolset present under
  VS 2026 but not in the default shell environment).

## Network isolation (2026-11-17)

* **Incident:** once the title's online subsystems started (first reached on 2026-11-17 when
  shader translation got the game past pipeline creation), the title issued real WinHTTP
  requests that succeeded — at least analytics to `datarouter.ol.epicgames.com`. That
  violated the project rule against contacting Epic production services. Runs before the
  guard below (a handful, a few minutes each) had network access.
* **Mitigation:** `compat-common::net_guard` now patches, inside the title process, the
  Winsock name-resolution/connect/sendto entry points and WinHTTP connect/open/send so they
  fail locally (installed on the first compat-layer call, i.e. before the online subsystem
  starts; verified: "Unable to create WinHttp Connection, failing request").
* **Known gaps:** `ConnectEx` to a literal IP is not intercepted; isolation is in-process
  only (no OS firewall rule — the session is not elevated). A proper sandbox (AppContainer
  without network capability, or a firewall rule) would be stronger.

## Audio

* `acphal`: `IAcpHal` is an inert object (no SHAPE DSP): The recorded retail lobby is silent. Coin Run’s XAudio2 PCM playback is verified separately by stereo loopback.

## Shader translator (tuxbox-xshader) — unverified assumptions

* Typed-buffer element type (float vs integer) is inferred from use (`infer_tbuf_types`):
  linear scan, majority vote, float when there is no evidence. Agrees with the PC build's
  declarations on the comparable pairs (`xpair --decls`), but it is a heuristic. Signed vs
  unsigned is not recovered reliably (declared `uint4` unless a signed operation is seen);
  a `uint` declaration over an `SINT` view is formally a type mismatch in D3D12.
* Typed *textures* are always declared float; the PC reference shows a few `Texture2D<uint>`
  / `Texture3D<uint>` (7 of 4,553 pairs) that are therefore wrong.
* Register slots differ between the Xbox and PC compilers in a sizeable share of shaders, and
  the pairing key (stage + layout hashes + signature names) often pairs different
  permutations; reference comparison by execution needs a stricter key.
* **Structured-buffer element size is a heuristic** (`sbuf_words`): 1 dword touched → 4
  bytes, 2..7 → 16, else rounded up to 16. Derived from one title's PC build. A buffer with
  stride 8/12/20/…, or a 16-byte buffer of which a shader reads only the first dword, will be
  addressed wrongly and silently. Proper fix: the runtime knows the real stride when the
  view is created; either validate it against the bound shader and report mismatches, or
  feed strides to the shader. Not started.
* `precise` is applied to position outputs only; other depth-equality dependencies (e.g.
  pixel-shader depth output) are not covered.
* Scalar buffer loads (`s_buffer_load`) are read as structured-buffer accesses only when the
  descriptor is unambiguously from the SRV/UAV table; with several candidate descriptors they
  are still treated as constant-buffer reads, which is wrong if one candidate is a buffer.
  The structured read declares a 4-dword element and indexes past it when the real stride is
  larger (works with DXC's raw-buffer lowering; not guaranteed by HLSL).
* Indexed constant-buffer arrays assume a 16-byte element stride.
* Compute thread-group id registers are assumed to follow the user-data SGPRs.
* PS entry ABI assumes perspective-centre barycentrics in v0/v1 and position in v2..v5.
* Inter-stage signatures: graphics pipelines keep the original producer when the
  compiled registers and masks agree. On a packing mismatch, TuxBox retries with
  the pixel shader's interpolation modifiers. Pairs where neither layout matches
  still need explicit signature packing.
* DXC uses host wave operations for `v_readfirstlane` and `v_mbcnt`. Guest and host
  wave widths can differ. A scalar population count of EXEC still sees the invocation's
  projected mask. The SM5 fallback retains its single-lane approximations.
* Coin Run validates synthetic graphics output against reference pixels. Retail
  shader equivalence still needs matched reference captures.

## Placed textures / texel mirroring (2026-11-17)

- **Resource states.** The mirror follows explicit legacy transition barriers on wrapped
  command lists in submission order. It transitions each changed subresource to COPY_DEST
  and restores its previous state. A pending split barrier defers that subresource.
  Cross-queue fence handoffs and implicit promotions from command usage remain untracked.
  The mirror defers a mip last transitioned on another queue until it observes a transition
  on the presenting queue. Coin Run exercises two mips on one direct queue with `--state-test`.
- **No write notification.** Changes are found by hashing title memory each frame under a
  byte budget (96 MiB new, 8 MiB re-check). A changed texture can lag by several frames, and
  the scan costs CPU every frame. Write-watch (`MEM_WRITE_WATCH`) would be the proper fix.
- **Racy reads.** Title threads write while we read (repaired on a later frame); a decommit
  racing the read would fault.
- **Streamed mips.** Coin Run’s `--stream-test` verifies that an interior hole in the
  CPU placement defers a mip upload. Mapping the remaining pages makes that mip available
  on the next tested present. Eviction and reload with different data at the same address
  are covered. The test selects the resident mip explicitly. A retail title’s mip selection
  during streaming and concurrent page removal during an upload remain unverified.
- **Not mirrored:** Texture3D, multi-plane formats, render-target/depth placed textures.
- **Layout source.** Element offsets come from the dump's real `xg_xs.dll` (vtable slots
  verified by probe for layout 261 only; other tile modes untested).
- **Extended sampler (slot 61).** Fields +0x3C/+0x54 of the Xbox desc are ignored.
- **XALLOC_ATTRIBUTES** bit layout is recalled from the public XDK header and matches the two
  attribute values seen; not checked against a header in this repo. Page size / memory type
  fields are ignored.

## Network redirect mode (2026-11-17)

- The in-process guard defaults to offline. The historical retail experiment used an explicitly selected private server. The development launcher now uses the same configuration as the runtime.
- Not covered: `ConnectEx` to a literal IP, any resolver path that bypasses the hooked
  ws2_32 / WinHTTP exports, other processes.
- The Host header and TLS server name become the redirect host; the originally requested
  host name is not forwarded to the server (the server cannot tell which Epic service a
  request was meant for except by path).
- Non-WinHTTP TLS clients in the title (if any) will see a certificate for the redirect
  host while expecting another name; untested.
- Literal-IP destinations other than loopback / the redirect host's addresses are refused,
  including LAN addresses.

## Shader translator (2026-11-17, offline pass)

- "Translates" and "compiles" are established for the archive (sweep pending completion);
  **semantic correctness is not**. No translated shader has been compared against reference
  output.
- Structured-buffer accesses with a run-time byte offset assume the declared element size
  equals the view's stride.
- Optimistic descriptor join: a register holding a descriptor on one incoming path and
  unknown on the other is treated as the descriptor.
- Wave reductions map to `WaveActiveMin/Max`; host wave composition differs from the
  console's 64-lane waves (as HLSL permits). FXC fallback cannot compile these.
- Image atomics other than add/umax, and reductions other than f32 min/max, are refused.
- DXC-compiled graphics pipelines are untested in the game.

## Shader outputs and texel types (2026-11-19)

- **Colour-export tail is assumed to be replaced per target format on console.** The
  translator exports the operands of the two pack instructions in front of a compressed
  colour export instead of the packed halves. Evidence: the same tail appears on `uint4`
  outputs, where the literal behaviour cannot work. The replacement itself was not observed.
  Consequence if wrong: float targets get more precision than the console; no other effect.
- **Texel type of textures is inferred from use** (integer vs float), like typed buffers.
  No evidence = float. Agrees with the PC declarations on every paired shader checked, but
  the pairing covers a fraction of the archive.
- **Binding trace cost.** Diagnostic only; it holds a reference on every buffer and a map
  entry per descriptor. Not suitable for normal runs as written. The same tracking is what
  a run-time check of structured-buffer strides would need.

## Vector-ALU semantics (2026-11-20)

- **Covered by the GPU conformance test** (`tests/valu_conformance.rs`, 263 cases):
  per-instruction behavior on one host GPU, including two SDWA word-select comparisons
  and a pair of CMPX instructions that successively mask lanes. Gaps include other SDWA
  forms, general DPP patterns, dynamic lane indices, and interactions inside larger shaders.
  Coin Run checks constant-index lane spills, relative VGPR moves, a 128-thread LDS
  exchange and selected interpolation forms. Through Xbox compute PSOs, Coin Run also checks
  24 scalar comparisons and 23 scalar arithmetic, selection, move and sign-extension
  instructions. The arithmetic probes read back both the result and the scalar condition
  flag, including carry and borrow across two words. Coin Run also checks both halves
  of scalar left shifts and bitfield masks, and 32-bit signed and unsigned field extraction.
  Its seven branch probes use one active lane per workgroup. Branches with divergent
  lanes and scalar 64-bit right shifts still need dedicated probes. The local PC-transfer
  tests cover a GETPC plus constant displacement. Reciprocal exception flags and
  concurrent stores to adjacent LDS halfwords remain untested. Cache probes measure
  cross-wave UAV visibility and register preservation. Hardware cache levels and prefetch
  distance remain unverified. [Coverage report](../tools/progress/coin-rdna2-coverage.tsv)
- **Fused vs. unfused multiply-add is not controlled.** HLSL has no single-precision FMA;
  `a * b + c` may or may not be fused by DXC/the driver, while the console fuses `v_fma*` /
  `v_fmac*` and does not fuse `v_mad*` / `v_mac*`. The test accepts either.
- **DXC's default floating-point assumptions.** Shaders are compiled without `-Gis`; the
  conformance test caught `x == x` being folded (now avoided), and other NaN-sensitive
  rewrites inside larger shaders cannot be excluded. Compiling with `-Gis` (and adding the
  flag to the cache key) is the general fix; its cost and effect in the game are unmeasured.
- **f32 denormals** are left to the host implementation (inputs with denormals are not
  tested). The console's denormal mode for these shaders is unknown.
- **`v_cmpx_*` write only EXEC**, not VCC/SDST. Inferred from LLVM's GFX10 encodings and
  the ISA guide ("write exec"); not observed on console. If they also wrote VCC, a shader
  reading VCC after a CMPX would differ.
- **The console ISA is a mix of GFX10.1 and GFX10.3** (see the 2026-11-20 log): an opcode
  valid in only one reference is accepted. An opcode whose meaning differs between the two
  would be mistranslated; none was found among the table entries.
- **Transcendentals** (`exp`, `log`, `rcp`, `rsq`, `sqrt`, `sin`, `cos`) match within the
  D3D minimum precision, not bit-exactly; `sin`/`cos` only for |x| ≤ 64 turns (the console's
  results for large arguments are not reproduced).

## Vector-memory semantics (2026-11-20)

- **Covered by `tests/mem_conformance.rs`** (71 cases, including a packed load followed by integer arithmetic).
  Not covered: image atomics, `image_get_resinfo` / `image_get_lod`, MSAA loads, 3D / array
  / cube gathers, `_c` gathers, depth-compare with LOD or bias, filtering other than point,
  border colours, sRGB / normalised / compressed formats (all cases use 32-bit float
  texels), buffer atomics, byte / short buffer ops, MUBUF with OFFEN, constant-buffer
  MUBUF.
- **Cube arrays are not handled.** Sampled cube images address a face as `slice * 8 + face`
  (ISA §8.2.4); the translation declares `TextureCube` and `tb_cube_dir` reads faces 0..5.
- **D16 integer formats and TFE/LWE/LDS are refused**, not translated. A shader using them
  fails translation with a message naming the bit.
- **D16 rounding** of float results is round-to-nearest-even of the D3D f32 result. For
  formats with more precision than f16 this matches a direct hardware conversion only if
  the hardware also rounds to nearest even (not documented). The high half of the last
  VGPR with an odd component count is written as 0; the hardware's value is unspecified.
- **UNORM sampling** normalises by mip 0 of the view; the console's behaviour for
  unnormalised coordinates on mipped views, and samplers that force unnormalised
  coordinates through the S#, are not modelled.

## User-language override (2026-11-21)
- `locale=` changes the user-language queries only. The geographic region
  (`GetUserGeoID`), the system locale and explicit per-thread UI-language settings keep the
  host's values. GDK `XPackageGetUserLocale` is not covered (no examined title uses it).
- The hooks patch `kernelbase` entry points in place. A host whose exports are too short for
  the patch (checked per function) keeps the host answer for that query and logs it.

## Map of GPU-only resources (2026-11-21)

The verified helper pattern is limited to a zero fill immediately after committed
resource creation. General CPU access to GPU-only resources still needs synchronized
readback and upload, including Xbox subresource layouts and queue state. The initial
retail report did not include the executable timestamp and call-site RVAs needed to
enable and retest the scoped workaround. [Configuration](RUNNING.md#gpu-resource-zero-initialization)

## Resource states (2026-11-21)
- Buffer-only read bits are dropped from texture transitions; other console-only state
  combinations, if any, are not translated. Root constant-buffer views at addresses that
  are not 256-byte aligned are passed through (accepted by the console, invalid on PC).

## GDK wrapper discovery (2026-11-21)
- Wrappers are recognised by one instruction shape (interface query, vtable call, release)
  inside a `.pdata` function. A wrapper that is inlined or compiled in another order is not
  found; the API then stays with the host runtime and the install log names it.
- Key names come from one title's symbol table. Wrappers for slots without a name are only
  reported: slot 0x68 of `XUser` in a second title, and slot 0x30 of a device-association
  interface with a third IID in Fortnite. Their APIs are not identified.
- `XUserFindForDevice` assigns every input device to the one local user. Several local
  users are not modelled.
- The XStore layer still uses build-specific addresses and is installed only for the
  recorded Fortnite build.
