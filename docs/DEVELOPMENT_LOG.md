# Development Log

All times UTC. This log is append-only and records what was actually done, run, and observed.

---

## 2026-11-16 — Session 1: Reconnaissance + toolkit bootstrap

### Environment (observed)
- **OS:** Windows 11 Home 10.0.22631 (x64)
- **CPU:** AMD Ryzen 7 5700G (8C/16T)
- **RAM:** 47.6 GB
- **GPU:** NVIDIA GeForce RTX 5060 Ti (primary); a "Meta Virtual Monitor" virtual adapter also present
- **Disk:** D: has 321.7 GB free (project lives on D:); C: 127 GB free
- **Toolchains found:**
  - Rust `rustc 1.99.1`, host `x86_64-pc-windows-msvc`, via rustup. Build+link+run smoke test PASSED.
  - MSVC VC tools: VS 2026 Community (`18.9.12105.275`), toolset 14.16.27023 at
    `C:\Program Files\Microsoft Visual Studio\18\Community\VC\Tools\MSVC\14.16.27023` (also VS 2017 present).
  - `.NET 10.0.400` (`dotnet`)
  - `git`, `node`
  - **No usable Python** — the `python.exe` on PATH is the Microsoft Store App Execution Alias stub, not a real interpreter.
  - `cmake`, `ninja`, standalone `clang`, `dumpbin` not on PATH.
- **MCP reverse-engineering servers available:** `rea` and `ida`. These are the primary binary-analysis tools once the real executable is extracted.

### Actions
1. Inspected working directory: contained only `CLAUDE.md` (not a git repo).
2. Created input/analysis/docs/src layout.
3. Probed the supplied archive URL via HTTP HEAD:
   - Status 200, `Content-Length` = **27,385,950,541 bytes (25.51 GB)**, `Content-Type: application/zip`, `Accept-Ranges: bytes` (resumable).
4. Wrote `input/original/download_metadata.json`.
5. Started a resumable background download with `curl.exe -L -C - --retry 5`. **(in progress)**
6. Chose Rust for the binary toolkit (see ADR-0001).

### Status of milestones (CLAUDE.md §12)
- [~] 1. Archive downloaded and checksum recorded — download IN PROGRESS (25.5 GB).
- [ ] 2. Dump extracted and inventoried.
- Remaining milestones blocked on the download completing.

### Next priorities
- While download runs: build + test the PE/COFF parser against known Windows PEs (does not need the real binary).
- On completion: verify SHA-256, extract safely, inventory, identify main executable + modules, parse it, enumerate imports to find Xbox/GDK dependencies.

### Progress during the download (same session)
- **Built `tombox-pe` + `pe-inspect`** (zero-dep PE32+ parser). 8 tests pass incl.
  real `kernel32.dll` (1671 exports, `CreateFileW` present, AMD64/PE32+), a no-panic
  sweep over random / PE-shaped / truncated inputs, and a DIR64-relocation assertion.
  Verified `pe-inspect` on a real binary: full header/section/import/dir/TLS/reloc report + valid JSON.
- **Studied AnyPS5** (delegated Explore agent; `docs/ANYPS5_STUDY.md`). Confirmed a
  static-relink + reimplemented-syslib design. Drove **ADR-0002**: native x64 execution
  with import redirection, not emulation. Noted AnyPS5 avoids MSVC only due to SysV-ABI
  catch handlers — not a constraint for Xbox, so Rust/MSVC is the right fit.
- **Verified public Xbox facts** (web): GDK D3D12.x, MicrosoftGame.config/MSIXVC
  flat-file layout, DXIL+offline-precompiled shaders — specifics NDA-gated → will come
  from binary evidence. Wrote `docs/XBOX_RESEARCH.md` with confidence tags.
- **Built `tombox-archive` + `zip-inspect`** (zero-dep, ZIP64-aware central-directory
  reader + extraction safety validator). 9 tests pass: ZIP64 64-bit size resolution,
  path-traversal / absolute / drive-letter / UNC / symlink blocking, bomb-ratio warning.
  Verified on a real `Compress-Archive` zip. **This gates the 25 GB extraction.**
- **Wrote docs**: README, ARCHITECTURE, COMPATIBILITY, TECHNICAL_DEBT, FORTNITE_ANALYSIS,
  DECISIONS (ADR-0001/0002), ANYPS5_STUDY. Initialized local git (game data/refs/target excluded).
- **17 tests total, all passing.** Toolchain: full Rust build/link/run confirmed; MSVC
  toolset located (VS 2026, 14.16).

---

## 2026-11-16 — Session 1 (cont.): download complete + real binary analysis

### Download & verification
- Archive downloaded in full: 27,385,950,541 bytes (size matches server exactly).
- **SHA-256 (archive):** `433E7B3B65C0C51A4F20001721F7CC9FDA9AE97A0022A7F7B5AA94914BEFCA95`
- `zip-inspect` on the real archive: ZIP64, 834 entries, **safe** (0 blocking findings).

### Extraction (selective, safe)
- Vetted first, then extracted **code only** (Binaries + System32 DLLs/EXEs + manifests +
  symbols = ~390 MB) via .NET ZipFile with a path-traversal guard; skipped 24 GB of assets.
- Structure: `EmbeddedXvd/` (Xbox GameOS system image, 276 System32 DLLs) + `Mount/`
  (game package). **Loose decrypted files — not an encrypted XVC** (premise confirmed).

### Main executable (parsed with pe-inspect)
- `FortniteClient-XSX-Shipping.exe`, 150 MiB, **PE32+/AMD64**, WINDOWS_GUI, MSVC linker 14.24,
  entry RVA `0x229a298`, image base `0x140000000`, ASLR/DEP, **2,045,522 DIR64 relocations**,
  TLS present, 8 sections incl. Xbox-specific **`.xbld`**.
- **SHA-256 (EXE):** `0B3D2E5968B0913EB01F52612793BFCC1984FF9764BD15D1C5AABF4830FCC3A0`
- **67 import modules / 670 functions.** ~52 are standard Windows API-Sets/UCRT (BindOS
  candidates). Reimplementation list is short: `d3d12_xs`, `xmem`, `xg_xs`, `GameInput`,
  `XAudio2_9`, `AcpHal`, `XCRDAPI`(DirectStorage), `dxcompiler_xs`, + dynamic `xgameruntime`.
- Analyzed each Xbox system DLL's exports/imports (`analysis/syslib/*.json`): `d3d12_xs` is
  a 33 KB **thin shim** exporting standard D3D12 COM + Xbox ext; `xgameruntime` exports only
  `QueryApiImpl` dispatch and is **dynamically loaded** (not a static import).

### Shaders
- **DXBC/DXIL confirmed** (not alien microcode): `dxcompiler_x/xs.dll` (DXC) + `dxbc2dxil_xs.dll`
  present; EXE imports `DxcCreateInstance`. Top graphics risk (R1) downgraded.

### Symbols
- `FortniteClient-XSX-Shipping-Symbols.bin` = UE console symbol DB, decompresses to 118 MB of
  demangled C++ names. Startup chain: `GuardedMain`→`FEngineLoop`→`RHIInit`→`FD3D12DynamicRHI`.

### Milestones reached (CLAUDE.md §12)
- [x] 1 Archive downloaded + checksum. [x] 2 Extracted + inventoried. [x] 3 Main exe + modules
  identified. [x] 4 Executable parsed/validated. [x] 5 Dependencies/imports mapped.
- [~] 6 Xbox loader requirements: substantially understood (dynamic xgameruntime, `.xbld`,
  import redirection). [~] 7 Architecture: ADR-0002 confirmed by evidence.

### Next priorities
1. Decode `-Symbols.bin` address↔name map; inspect `.xbld`.
2. Enumerate GDK `QueryApiImpl` surface via symbol-labelled call sites (REA/IDA).
3. Begin the loader PoC: map + DIR64 reloc + import table build with redirection + fail-loud
   unsupported-feature table (now evidence-justified).

## 2026-11-16 — Session 2: real binary + first compat DLLs

### Archive (downloaded, exact size match 27,385,950,541 bytes)
- `zip-inspect`: ZIP64, 834 entries, 0 symlinks/traversal → **safe**. SHA-256 in `input/original/sha256.txt`.
- Structure: `Mount/` (game) + `EmbeddedXvd/` (Xbox GameOS System32 DLLs) + `era.vbi`. NOT an encrypted XVC — loose decrypted files (premise holds).
- `MicrosoftGame.config`: main exe `FortniteClient-XSX-Shipping.exe`, `TargetDeviceFamily="Scarlett"`, `RequiresXboxLive=true`.
- Selectively extracted ~375 MB of code (Binaries + System32 + manifests); skipped 24 GB of `.ucas/.pak/.mp4` assets.

### Main EXE (`FortniteClient-XSX-Shipping.exe`, 157 MB)
- PE32+ / AMD64, 8 sections, LARGE_ADDRESS_AWARE, ASLR/DEP/HIGH_ENTROPY_VA, subsystem WINDOWS_GUI, linker 14.24 (MSVC2019), entry 0x229a298, image base 0x140000000, 2,045,522 base relocs.
- **67 import modules / 670 functions.** ~52 are standard Windows API-Sets/UCRT (bind to real Windows). Delay import: GameChat2.dll.
- **Xbox-only modules (the reimplement list):** d3d12_xs (6), xmem (10), xg_xs (1 `XGCreateTextureComputer`), GameInput (ord #1), XAudio2_9 (ords #1-6), AcpHal (6), XCRDAPI (`DStorageGetFactory`), dxcompiler_xs (`DxcCreateInstance`).
- `FD3D12DynamicRHI` present, `FScarlett`/`ScarlettRHI` absent → D3D12.x handled inside the normal D3D12 RHI → **D3D12.x is a dialect of PC D3D12**.
- Symbols: `Mount/Symbols/*-Symbols.bin` = UE console symbol DB, `[magic 0x076b85ee][size][zlib]` → 118 MB of demangled C++ names (GuardedMain/FEngineLoop/RHIInit present). Record layout `[rva][size][?]`×1,245,949 + length-prefixed string table (full decode deferred; names confirmed present).

### VALIDATED ON REAL HARDWARE (user)
- User launched the EXE natively on Windows: it loads to import resolution and only fails on missing Xbox-only DLLs → **confirms ADR-0002 (native x64 + import redirection), no CPU emulation.**
- Build 1 DLLs (xmem, d3d12_xs, xg_xs, xcrdapi): exports verified == imports via our own `pe-inspect`; static CRT; only import kernel32/ntdll/apisets. xmem heap-backed (real), rest fail-loud stubs, all call-logged to `%TEMP%\tombox_compat.log`.
- After build 1, loader advanced and requested `dxcompiler_xs.dll` + `AcpHal.dll` → Build 2 shipped (both verified).

### Next
- Await user's next-missing-DLL + the compat log (call order) → implement earliest-called Xbox API for real; D3D12 device creation is the expected wall (needs the graphics backend).

## 2026-11-16 — Session 2 cont.: boots to GDK runtime init (major milestone)

### Milestone reached: the real game runs its own engine code
After the compat DLLs + two xmem fixes, `FortniteClient-XSX-Shipping.exe` loads
natively and executes its own startup code. Verified via a purpose-built debug
tracer (scratchpad `mdbg.rs`: DEBUG_ONLY_THIS_PROCESS loop capturing
OutputDebugString, exceptions, exit-sink stack).

### Two real bugs found + fixed from runtime evidence (not guesses)
1. `XMemVirtualAlloc` passed Xbox allocation-type bits (0x20000000) to Windows
   `VirtualAlloc` → read as MEM_LARGE_PAGES → ERROR_INVALID_PARAMETER → null →
   UE memory init failed. Fix: mask to Windows-valid alloc-type bits.
2. `XMemVirtualAlloc` committed pages as PAGE_NOACCESS (Xbox prot value 0x1 passed
   through) → game wrote to committed page → ACCESS_VIOLATION (confirmed via
   minidump + disasm: `mov [rax],r15`, rax = our returned pointer). Fix: committed
   pages get READWRITE (EXECUTE_READWRITE if execute requested); reserve stays NOACCESS.

### Current wall: Xbox GDK runtime (XGameRuntime) init → clean exit(1)
OutputDebugString trace shows the GDK runtime bootstrapping its service objects
(`SErrorImpl`, `SUsersImpl`, `SGameImpl`, `SUserDeviceAssociationsImpl`,
`SMultiplayerNetworkingImpl`, `SSystemImpl`, `SFeatureImpl`, `SAsyncImpl`,
`SPersistentLocalStorageImpl` — all "Created"), then **all "Destroyed"** and the
process exits 1. `XERROR: HR:80070006 ... TaskQueue` during teardown. Graphics
(`d3d12_xs`) is NEVER reached — this is platform/GDK init, before the RHI.
Exit-sink stack walk: `main`/`WinMain` already returned (CRT teardown), i.e. a
**clean decision to quit**, not a crash.

### Why: the title needs the Xbox GameOS environment
- `MicrosoftGame.config` TargetDeviceFamily="Scarlett"; `appxmanifest.xml` has
  `<uap7:OSPackageDependency Name="Microsoft.GameOs" .../>` and an Xbox proxy
  entry point. `XGameRuntimeInitialize` expects the console runtime/identity.
- Package registration ruled out: `Add-AppxPackage -Register` → 0x80073CFF
  (no sideloading license); and the Microsoft.GameOs dependency can't be met on
  desktop Windows anyway.

### Next phase (decided): XGameRuntime / GDK compat shim
Reimplement the Xbox GDK runtime surface (XGameRuntimeInitialize succeed, XTaskQueue,
XAsync, XUser, XGame, PersistentLocalStorage, etc.) — the platform-services layer
anticipated by CLAUDE.md §B. Same incremental method: shim → run → read the call
log → implement the next required function. This is a substantial new subsystem.

### Tooling added this session
- `mdbg` debug tracer (OutputDebugString + exception + exit-stack capture) — to be
  formalized into the repo as `tombox-trace`.
- `mdparse` minidump parser (exception code/address/faulting module).

## 2026-11-16 — Session 2 cont.: root cause pinned to the instruction (UE project load)

### Milestone: the game boots into UE engine init; exit cause fully traced
Built a self-contained x64 disassembler (`crates/tombox-disasm`, iced-x86 + tombox-pe
RVA mapping + symbol resolution) and a full symbol resolver from the shipped
`FortniteClient-XSX-Shipping-Symbols.bin` (1,245,949 named functions; verified: EXE
entry RVA 0x229a298 -> WinMainCRTStartup). Extended the debug tracer (`mdbg`) with
EXE breakpoints + register/stack/FString capture + a continuing CreateFileW probe.

### The exit path, traced end to end (all confirmed at runtime)
`WinMainCRTStartup` -> `WinMain` (0xb45c64): calls `XGameRuntimeInitialize` (succeeds),
`GuardedMainWrapper` -> `GuardedMain` (0xb46088) -> `EnginePreInit` (0x2bcd12c) ->
`FEngineLoop::PreInitPreStartupScreen` (0xe6d82c) **returns 1** (captured eax=1 at
EnginePreInit's test). Bisected PreInitPreStartupScreen by runtime breakpoints to:
`IProjectManager::Get` -> virtual `[+0x10]` = **`FProjectManager::LoadProjectFile`**
(RVA 0x15071cc) returns FALSE -> PreInit bails before RHI (no d3d12_xs ever called).

### Root cause (exact)
`LoadProjectFile` -> `FProjectDescriptor::Load` (0x15072e4) ->
`FFileHelper::LoadFileToString("../../../FortniteGame/FortniteGame.uproject")`
**returns FALSE** (captured al=0). The game cannot read its **project descriptor** and
aborts. Notes:
- No `.uproject` exists anywhere in the package (archive, EmbeddedXvd, EXE strings).
- Placing a valid `.uproject` at the resolved relative path (Mount/FortniteGame/) did
  NOT satisfy it; the read still fails.
- A `CreateFileW` trace caught **zero** file opens -> the GDK/Xbox platform file layer
  uses a different API (almost certainly **`CreateFile2`**) and/or a package-mount
  namespace, not plain Win32 loose-file reads.

### Interpretation
On real Xbox the project descriptor is served by the console package file system
(`FXboxPlatformFile`) from the cooked package; our loose-file PC layout + the absent
Xbox platform-file semantics mean the read fails. **Next subsystem: Xbox platform
file / content mounting** (how the title resolves and reads packaged files), to make
`FFileHelper::LoadFileToString` succeed for the descriptor (and later, pak/IoStore).

### Tooling note
`crates/tombox-disasm` is committed. The diagnostic tracer (`mdbg`), minidump parser
(`mdparse`), symbol resolver (`symres`) and export/IAT readers currently live in the
session scratchpad and should be formalized into the repo (`tombox-trace`, `tombox-sym`).

## 2026-11-16 — Session 2 cont.: REAL D3D12 DEVICE CREATED; full engine init runs

### The game now runs its entire Unreal Engine init on PC
With G:\ mapped to the package (subst) and the compat layer, FortniteClient-XSX
boots through its real init, with full UE logging:
- FEngineLoop PreInit (project descriptor now loads from G:\FortniteGame\)
- IoStore/DirectStorage (XCRDAPI DStorageGetFactory -> forwarded to real PC
  dstorage.dll with PC IID substitution; Xbox/PC IDStorageFactory compatible)
- APU audio heap (AcpHal ApuCreateHeap/ApuAlloc/ApuHeapGetState -> real host memory)
- Chaos physics, localization, LogOnline/MCP, GDKInput (5 controller devices),
  Slate (UI user)
- D3D12RHI: XGCreateTextureComputer (REAL xg_xs.dll used as-is — pure RDNA2
  tiling math, self-contained: imports only msvcrt + api-ms-win-core + our xmem)
- **D3D12XboxCreateDevice -> created a REAL PC D3D12 device** (host d3d12.dll;
  Xbox device IID == PC ID3D12Device {189819f1-...}, hr=S_OK, live device handle)

### Current wall: D3D12.x device interface divergence
`RootDevice->QueryInterface(__uuidof(IDXGIDevice))` fails: Xbox D3D12.x merges the
DXGI device into ID3D12Device; PC keeps them separate. First point where D3D12.x
needs a **wrapper/translation**, not a plain forward. Next phase: a COM proxy for
the device (handle QI for IDXGIDevice + Xbox-specific methods, forward the rest),
then command queues, swapchain/present, resources, PSOs, shaders.

### Key technique discovered
Some Xbox DLLs are pure computation and run on PC as-is (real xg_xs.dll). Others
forward to PC equivalents with IID substitution (DStorage, D3D12 device). Only the
hardware/semantics-divergent surface needs real translation.

### Milestones (CLAUDE.md §12)
- [x] 12 Actual Fortnite reaches meaningful initialization — ACHIEVED (full UE init).
- [~] 11 Graphics device initialized — device CREATED; post-device RHI setup pending
  the D3D12.x wrapper.

## 2026-11-16 — Session 2 cont.: real GPU driven; D3D12.x translation layer underway

### The real binary drives our RTX 5060 Ti through D3D12 setup
FortniteClient-XSX runs its full engine and now performs real GPU work via a
D3D12.x -> PC D3D12 translation layer (crates/compat/d3d12_xs):
- Device wrapper (COM): QI returns self for ID3D12Device..Device8 and synthesizes
  the Xbox "DXGIDevice" (GetAdapter -> real PC IDXGIAdapter -> game reads RTX 5060 Ti,
  7891MB VRAM). 160-slot vtable: 0-43 forward via naked thunks; 44+ are Xbox
  extensions -> fail-loud E_NOTIMPL stub unless intercepted.
- Verified translations: CheckFeatureSupport (Xbox 64B vs PC 60B struct-size
  negotiation), CreateDescriptorHeap (wraps heap), CreateCommandQueueX slot 48
  (Xbox desc -> PC CreateCommandQueue, created a real compute queue hr=S_OK).
- Heap wrapper: fixes the Xbox(RAX)-vs-PC(sret `fn(this,Handle* retval)`) return
  convention for GetCPU/GPUDescriptorHandleForHeapStart.

### Current frontier: Xbox D3D12.x method signatures
Next blocker is the Xbox command-list create (device vtable slot 73): standard PC
CreateCommandList signature does NOT match (args mismapped -> E_INVALIDARG), so the
Xbox vtable relocates/extends methods and some take Xbox-specific signatures.
Resolving each needs the GDK d3d12_x headers or per-method RE. The remaining
graphics path (command lists, resource creation, PSOs/shaders, swapchain/present)
is a methodical per-method translation grind — the technique is proven; the volume
is large.

### Milestones
- [x] 12 meaningful initialization — done (full UE init + GPU device + queues).
- [~] 11 graphics device + required features — device/caps/heaps/queues done;
  command lists onward pending D3D12.x ABI reversing.

## 2026-11-16 — Session 2 cont.: real GPU command infrastructure up

### The game drives the real RTX 5060 Ti through D3D12 command setup
Continued the D3D12.x translation by reversing each Xbox method's ABI from its
call site (no headers needed):
- CreateCommandQueueX (slot 48): Xbox queue desc -> PC CreateCommandQueue. Real
  compute queue created (hr=S_OK).
- Command-list create (slot 73): reversed the Xbox signature from the call site —
  takes a params struct {Type@0, NodeMask@8} by pointer, not inline args. Translated
  to PC CreateCommandList (nodeMask must be non-zero). Real DIRECT list created.
- Close-on-create: PC CreateCommandList returns an OPEN list; the Xbox RHI expects
  it CLOSED then calls Reset() — closing it on create fixed an nvwgf2umx (driver)
  crash in FD3D12CommandListData::Reset.
- Descriptor-heap handle ABI: switched from wrapping the heap (which leaked our
  wrapper into real SetDescriptorHeaps -> driver crash) to patching the shared
  ID3D12DescriptorHeap vtable slots 9/10 in place (VirtualProtect) — heaps stay
  real, handle getters bridged (Xbox RAX vs PC `fn(this, Handle* retval)`).
- Robust 160-slot device vtable: standard methods forward, Xbox extensions default
  to fail-loud E_NOTIMPL stubs (no crashes), specific ones intercepted.

### Current frontier: command-list recording method slots
The game now Reset()s and records on the real command list, then its own Close()
fails -> the command-list recording methods (barriers, descriptor binds, draws —
~50) are at Xbox vtable slots that differ from PC, corrupting the recording. Fix
is a command-list wrapper that remaps Xbox slots -> PC methods AND unwraps on
ExecuteCommandLists. That object-graph wrapping (command lists, resources, PSOs)
+ swapchain/present + shader handling is the remaining rendering-pipeline work —
proven approach, large volume.

### Milestones
- [x] 12 meaningful initialization.
- [~] 11 graphics device + features: device, caps, descriptor heaps, command
  queues AND command-list creation/reset all driving the real GPU; command
  recording -> present is the next phase.

## 2026-11-16 — Session 2 cont.: Xbox D3D12.x command-list vtable solved

### Root cause of the descriptor-table crash: a +4 vtable slot shift
Enabled the D3D12 debug layer (TOMBOX_D3D12_DEBUG) in the shim — its validation
messages flow through OutputDebugString into the tracer, turning opaque driver
crashes into precise errors. It reported #646 INVALID_DESCRIPTOR_HANDLE in
SetGraphicsRootDescriptorTable(root=2, handle=<stack ptr>). Patched the shared
command-list vtable to log binds + caller RVA: the caller was
FD3D12DescriptorCache::SetDescriptorHeaps calling command-list **slot 32** with
(NumHeaps=2, ppHeaps=&{srvHeap,samplerHeap}) — i.e. it was **SetDescriptorHeaps**
(PC slot 28), not SetGraphicsRootDescriptorTable (PC slot 32).

Reverse-engineered the Xbox command-list vtable from call sites in the Fortnite
RHI: slots 0..=26 are identical to PC (verified: Close=9, Reset=10,
DrawInstanced=12, DrawIndexedInstanced=13, Dispatch=14, SetPipelineState=25,
ResourceBarrier=26), then **4 Xbox-specific methods are inserted after PC slot
27**, shifting every method >=28 by +4 (SetDescriptorHeaps 28->32,
OMSetRenderTargets 46->50, all verified).

Fix (cmdlist_wrap.rs): build an "Xbox-layout" vtable whose slot i holds the real
PC method pointer for the mapped PC slot (identity 0..=26, stub 27..=31, i-4 for
>=32), then repoint each real command-list object at it. No thunks needed — the
object IS the real list, so the real method runs on the real `this`. Slots 0..=26
stay identical so the runtime's own QI/GetType/AddRef/Release still work at
ExecuteCommandLists time.

Result: **#646 gone, all D3D12 validation clean.** The game now records and
submits GPU work correctly. New blocker is unrelated: IoStore/.ucas content
containers fail to open (DirectStorage path), fatal at pak mount.

### Milestones
- [x] 11 graphics device + required features: device, caps, descriptor heaps,
  command queues, command lists, descriptor-table binding all correct on the
  real GPU with zero D3D12 validation errors.

## 2026-11-16 — DirectStorage container open + found incomplete extraction

### Xbox DirectStorage OpenFile IID substitution
With the command-list vtable fixed, the next fatal was IoStore content: the Xbox
IoStore backend (FXSXFileIoStoreImpl::OpenContainer) opens .ucas containers via
DirectStorage factory->OpenFile (slot 4, PC-compatible slot). It passes the Xbox
IDStorageFile IID, which PC OpenFile rejects (like the factory IID). xcrdapi now
patches the real factory's vtable slot 4 to substitute PC IID_IDStorageFile
{5de95e7b-...}. Result: OpenFile succeeds, containers open, pakChunkEarly-XSX
mounts, plugins mount, device-profile CVars apply.

### ROOT ISSUE found: the dump was only partially extracted
The remaining fatal (appError Line 644) is almost certainly missing content: the
original ZIP is 834 entries / 26.9 GB, but only 905 MB (302 files) had been
extracted — just the smallest paks (global, pakChunkEarly, pakchunk0). Missing:
pakchunk10-XSX.ucas (21 GB — the bulk of the game), pakchunk12/7/8/2/5/11, all
Movies, 468 files total. Verified the 302 present files are byte-complete (no
truncation). Extracting the missing 468 now (disk has 294 GB free).

## 2026-11-16 — Full content + physical memory + resource/texture creation

After extracting the full 26.9GB dump (was partially extracted), cleared a chain
of real blockers past engine init into GPU resource creation:

1. **XMemAllocatePhysicalPages / XMemMapPhysicalPages** (xmem.dll): were stubs
   returning null -> fatal "AllocateTitlePhysicalPages failed" (Line 644).
   Implemented commit-on-map emulation of Xbox 64KB title physical pages (grant
   PFN tokens on allocate; VirtualAlloc-commit the reserved VA on map). The
   transient memory allocator now initializes (1792 DRAM pages).
2. **sret device thunks** (device_wrap): GetResourceAllocationInfo (25) and
   GetCustomHeapProperties (26) return >8-byte structs by value, so `this` is in
   rdx (rcx = hidden return ptr). The generic rcx-forwarding thunk null-derefed.
   Added fwd_sret! thunks that redirect rdx.
3. **CreateCommittedResource (27)**: intercepted to substitute PC IID_ID3D12Resource
   (already matched, but robust) — real resources create (buffers hr=0).
4. **GetResourceAllocationInfo (25)**: the real D3D12Core method faults deep in its
   texture-tiling sub-function (vtbl+0x25829) for Xbox texture descs even after
   full desc sanitization (valid device, valid 1x1 BGRA desc). Compute the info
   ourselves from the D3D12 64KB standard-swizzle tile model, over-estimating so
   the pool never under-reserves. Texture allocation crash cleared.

### Current frontier: GPU device removal
With CPU-side resource/texture creation working, a GPU operation during resource
setup triggers DXGI device-removed ("GPU Crashed or D3D Device Removed", Line 656).
Likely the unified-memory gap: Xbox places GPU resources on app physical pages
(GPU-visible on console); our commit-on-map gives CPU VirtualAlloc memory the GPU
can't access. Investigating.

## 2026-11-16 — ★ FIRST WINDOW: real Fortnite XSX binary opens on PC ★

MILESTONE 13 reached. After the presentation-init stubs (S_OK), the real
FortniteClient-XSX-Shipping.exe **created and presented its own window** on the
desktop (titled "Fortnite", blank first frame) before continuing init — the
actual supplied Xbox Series console binary driving a live D3D12 swap chain on the
RTX 5060 Ti. Confirmed visually by the user.

### Async asset streaming (DirectStorage read path)
After the window, the IoStore I/O thread (FXSXFileIoStoreImpl::StartRequests)
crashed: it enqueues DirectStorage reads on a queue that was null, because we had
only substituted the PC IID on OpenFile. Added CreateQueue (factory slot 3) IID
substitution (PC IID_IDStorageQueue {cfdbd83f-...}). The queue now creates and the
game advances to FFortniteEarlyPreLoadScreen + FAssetRegistry startup + a WinHttp
service call.

### Current frontier: DSTORAGE_REQUEST translation
Real DStorageCore now fail-fasts in DStorageQueue.cpp:287 with E_INVALIDARG — the
Xbox DSTORAGE_REQUEST layout passed to IDStorageQueue::EnqueueRequest differs from
PC. Next: intercept EnqueueRequest on the queue vtable, translate the Xbox request
(source file/offset/size + destination) to the PC DSTORAGE_REQUEST, or service the
read directly. This is the asset-streaming subsystem that gates reaching the menu.

### Milestones
- [x] 13 actual game renders a first frame (window created + presented).
- [~] 14 main menu: blocked on DirectStorage request translation (asset streaming).

## 2026-11-16 — DirectStorage complete; content fully loads

Full Xbox DirectStorage -> PC translation now works end to end:
- Factory IID substitution for OpenFile (slot 4), CreateQueue (slot 3),
  CreateStatusArray (slot 5).
- DSTORAGE_REQUEST Xbox->PC field reorder (Dest before Source on Xbox).
- Reads serviced directly via Win32 (CreateFileW in OpenFile -> OS handle map;
  positioned ReadFile in EnqueueRequest) because DStorageCore's own read path
  faults on the Xbox-origin requests; the game's Submit still fires status/fence.

Result: all 27GB content streams in, pakchunk10 (21GB) mounts, all 12 bundles
install ("All Bundles Installs Complete!"), asset registry starts.

### Current frontier: IoStore package-store build
An async task lambda crashes (AV read null) in the FPackageId->FPackageStoreEntry
map build (TMapBase::FindOrAddImpl) — the entries buffer [rbp+0xA0] is null.
Post-content-load engine init; needs tracing where that buffer is set up (likely a
stubbed Xbox path returning null, or an IoStore package-store format detail).

## 2026-11-16 — Post-content crash: container header not decompressed

Traced the package-store build crash (async lambda in FPackageStore::LoadContainers,
deserializing FContainerHeader). Root cause identified precisely via a crash-frame
dump: the source buffer handed to the FMemoryReaderView contains ZLIB-COMPRESSED
data (magic 0x78 0x9c), and the game deserializes it raw -> first bytes read as a
package id/count -> null entries array -> AV read null. (The size field is fine at
0x10000; an earlier read of the 40-bit size looked wrong but the byte at +0x18 is 0.)

The buffer is memory-mapped (0x7ff4_xxxx_xxxx), NOT one of our 4 DirectStorage dest
buffers, so it comes from a read path (FIoDispatcherImpl::ReadWithCallback) where the
chunk's decompression isn't applied on PC. On Xbox this chunk is decompressed
(hardware decompression via the Velocity Architecture / DirectStorage) before
deserialization.

NEXT: trace the container-header read/decompress path; either the engine expects
DirectStorage to decompress (CompressionFormat in the request) and our self-serviced
reads must zlib-inflate, or the engine's own decompress step is being skipped. All
DirectStorage self-service reads verified complete/non-zero (not the cause of bad
data elsewhere).

## 2026-11-16 — ContainerHeader decompression/routing (precise localization)

Deepened the package-store crash analysis:
- The source buffer handed to the FContainerHeader deserializer is zlib-compressed
  (78 9c) and memory-mapped (0x7ff4_xxxx_xxxx).
- Our 4 DirectStorage reads are all uncompressed plaintext (verified: readable
  strings, CompressionFormat=0, src==dst size) -> NOT the cause.
- FCompression::UncompressMemory IS called with zlib (78 9c) data during load, so
  the zlib path works. ScatterBlock's synchronous UncompressMemory (0x5c0789) is
  NOT hit before the crash; decompression goes via another caller
  (FDecompressAsyncTask).
- Conclusion: the ContainerHeader's *decompressed output is not routed to the
  deserializer* — the lambda's captured buffer ([this+0x18]) still points at the
  compressed source. It's a read-callback buffer-plumbing difference, not a missing
  decompressor.

NEXT: add log-and-continue (single-step replant) support to mdbg (cfw_handle
already does the trap-flag dance) so every UncompressMemory call + result + caller
can be logged; then find where the ContainerHeader read hands the compressed source
to the callback instead of the decompressed buffer. Chunk type = 0x0A
(ContainerHeader), read via FIoDispatcherImpl::ReadWithCallback from
FPackageStore::LoadContainers.

## 2026-11-16 — ROOT CAUSE: compressed container header read without decompression

Found the exact data behind the package-store crash:
- The buffer deserialized as FContainerHeader is the raw contents of
  pakchunk9-XSX.ucas at file offset 0 (verified: the exact bytes the game
  deserialized, 789ced..., are at offset 0 of pakchunk9-XSX.ucas and NOWHERE
  else).
- pakchunk9-XSX.utoc: 433 compressed blocks, 1 method = "Zlib", block size 64KB.
  block[0] (offset 0) = {compSize 2707, uncompSize 65536, methodIdx 1=Zlib}.
  It zlib-inflates cleanly to a valid 64KB container header (starts with an
  FIoContainerId).
- So the container header IS correctly Zlib-compressed per the TOC, but the engine
  hands the deserializer the RAW compressed block — it is not decompressing it.
- global/pakchunk0/pakChunkEarly have UNCOMPRESSED headers (no 78 9c at offset 0),
  which is why they loaded and pakchunk9 is the first container to crash.
- FCompression::UncompressMemory is called during load but only from the .pak path
  (FPakUncompressTask); the IoStore ScatterBlock decompress (0x5c0789) is never hit
  before the crash. So the IoStore compressed-block decompression is not running for
  this container-header read.

This is the hardest blocker so far: it's not a shim-returns-wrong-value bug, it's
the engine's IoStore read/decompress routing differing on PC. Fix options:
  (a) make the engine's IoStore decompression run for this read (trace why
      ScatterBlock/mmap path skips it),
  (b) intercept the container-header file read and return inflated data (needs file
      I/O hooking, scoped to not double-decompress content),
  (c) rewrite affected .ucas/.utoc to store uncompressed headers (offset-shift the
      whole container — deterministic but heavy).

## 2026-11-17 — Shader format cracked; RDNA2 translator foundation built

Reached the game's own "Gaming UI" window (black). Root-caused the black screen: every
graphics PSO fails because the shaders ship as Xbox-native code, DXIL stripped. Reversed the
full chain with evidence: `DXBC → DXIL(NOBC) → XDAT v3 → zlib(78 9c) → FSSM(v0x101) → native
RDNA2 ISA` (confirmed by s_waitcnt/s_endpgm/s_code_end/s_cbranch markers). Not encrypted.

Decision (user): build the real translator. Architecture: runtime JIT at PSO-create (Xbox
device slot 60/55), per-hash disk cache, backend RDNA2 → HLSL → dxc → DXIL. Rationale: RDNA2
VGPR ops are per-lane = per-thread, mapping cleanly to HLSL; dxc handles DXIL emit+signing.

Built `crates/tombox-xshader` (2 passing regression tests):
- `extract()`: DXBC→XDAT→zlib→FSSM→ISA, bounds-checked, validated against RDNA2 markers.
- `rdna2::disassemble()`: length-accurate decoder + full operand decode (SOP*/SMEM/VOP*/VINTRP/
  EXP), self-validated by landing exactly on s_endpgm (VS 67 instrs/400B, PS 117/604B).

Next: VOP opcode-semantics table → RDNA2→HLSL emitter (straight-line first) → dxc compile →
wire into create_gfx_pso_x with caching + fallback. See docs/FORTNITE_ANALYSIS.md.

## 2026-11-17 (later) — network guard, frame pacing, first visible frame

**Network guard.** Once pipeline creation worked the title's online subsystems started and
issued real WinHTTP requests (analytics to `datarouter.ol.epicgames.com`, observed
succeeding). `compat-common::net_guard` now fails DNS/connect/sendto/WinHTTP in-process
(14 entry points); every run summary prints the guard status. Details and gaps in
`docs/TECHNICAL_DEBT.md`.

**Translator.** Block dispatcher for CFGs that are not nested loops, 64-bit vector shifts,
`flat_load_dword*` from embedded constant tables (only the table tail is emitted: FXC's
4096 indexable-literal limit). Global archive coverage 1410/1689 (83.5%).

**Command-list shim.** `FD3D12StateCacheBase::ApplyState` (EXE+0x4e0433) writes command
packets inline through `[list+18h]`/`[list+30h]` and only calls the vtable when the buffer is
full. The title now receives a zeroed shim object (write ptr == end ptr) whose thunks forward
to the real list; `ExecuteCommandLists` unwraps shims. Only that one inline site is verified.

**Frame pacing (`d3d12_xs/src/frame.rs`).** The stall after engine frame 8 was the game
thread waiting on a flip-synchronised render fence (UE watchdog `FThreadHeartBeat` then
killed the process ~90 s later). Device slots 78/79/80/81 = SetFrameIntervalX /
ScheduleFrameEventX / WaitFrameEventX / per-token frame statistics, reconstructed from
`XboxCommonGDKUpdateFrameEvents`, `FD3D12Viewport::PresentInternal`,
`RHIWaitForFlip` and `GetFrameStatisticsForMostRecentFrame`. Implemented: interval timer,
flip-event signalling, tokens, statistics for tokens that reached PresentX.

**Result (verified, repeatable twice undebugged):** the title runs > 3 minutes without the
watchdog firing, the game thread is in `UGameEngine::Tick`, 4000–5400 PresentX frames, and
the host window shows drawn, animated content — a blue full-screen background with a yellow
rotating quad (`analysis/evidence/2026-11-17_first_visible_frame_{a,b}.png`). This is output
of the title's own draw calls through translated shaders.

**Not yet true:** no textures (device slot 44 CreatePlacedResourceX and slot 61 sampler are
still stubs, cmdlist slots 67/68/83 are no-ops), so logos/text are missing; no audio; one
debugger run showed a 14 s RHI-thread stall followed by a watchdog kill (not reproduced
undebugged; cause unknown).

Tools: `tools/dev/mdbg.rs` gained `MDBG_LIMIT` and `MDBG_HANGDUMP` (all-thread stack scan);
`tools/dev/hang.sh` symbolises the dump.

## 2026-11-17 (evening) — placed textures, title screen visible

**Slot 44 `CreatePlacedResourceX`.** Call site `FXboxCommonD3D12Adapter::CreatePlacedResourceX`
(EXE+0x5c4610) passes `(VirtualAddress, desc, InitialState, null clear, riid, ppv)` — the
public GDK signature. Caller `CreateVirtualTexture` reserves a 64 KiB-aligned block (streamed
textures) or takes memory from a small-texture pool / `XMemAlloc`. Implemented as a committed
PC texture plus a registry entry (`d3d12_xs/src/placed.rs`). ~5,100 textures are created on
the way to the title screen, all with Xbox layout value 261.

**Slot 61 extended sampler.** `FD3D12Device::CreateSamplerInternal` uses it only for
anisotropic filters, with a 0x6C-byte desc copied from the standard one; implemented as the
standard anisotropic sampler (two extra clamp fields ignored, meaning unverified).

**Texel mirroring (`d3d12_xs/src/upload.rs`).** The texel memory is in the console's tiled
layout and the title never announces writes. The real `xg_xs.dll` from the dump runs natively
and provides the layout: `tools/dev/xgprobe.rs` verified vtable slot 6
(`GetTexelElementOffsetBytes`) against slot 8 (`CopyIntoSubresource`) for every element of
five texture shapes. Slot 9 (`CopyFromSubresource`) returns E_INVALIDARG off-console (needs
an exclusive-lock object), so de-tiling is done from slot-6 offsets. Each PresentX hashes
title memory (new subresources first, uploaded ones round-robin, byte budgets) and re-uploads
changes through a private command list.

**`XMemAlloc` alignment.** 469 texture allocations use attributes 0x30804000; decoding the
XALLOC_ATTRIBUTES bit layout (alignment = bits 24..28, log2) gives 64 KiB. The shim ignored
it and returned heap pointers; it now page-allocates aligned requests.

**Result (one 170 s debugger run, 2,400 frames):** the host window shows the title's own
title screen — FORTNITE logo, yellow button with the X-button glyph, Unreal/Epic logos
(`analysis/evidence/2026-11-17_title_screen_logo.png`). The log reaches
`UFortUIStateWidget_Login::HandleLoginRequestComplete ... Unable to complete login`, which is
the expected outcome with the network guard on.

**Not yet true:** no text (font glyph textures are not arriving — path not yet identified);
no input, no audio; unmapped streamed mips are skipped (re-tried every frame); command-list
slots 67/68/83 are still no-ops; device slot 74 still a logging stub.

## 2026-11-17 (night) — text, texture copies, network redirect

**Command-list slots.** Xbox 83 = `CopyTextureRegion` + one trailing argument (always 0 at
`FD3D12TextureBase::UpdateTexture` and `FD3D12RHICommandInitializeTexture`), forwarded to PC
slot 16; Xbox 68 = `OMSetDepthBounds` (PC slot 62, inferred from the state cache's two
depth-bound floats); Xbox 67 = texture-cache flush, deliberate no-op. With the copies
forwarded the font atlas fills in: the title screen now shows "UNABLE TO COMPLETE LOGIN.
PLEASE TRY AGAIN LATER.", RETRY and "v14.50 (14617919)"
(`analysis/evidence/2026-11-17_title_screen_text.png`).

**Network policy change (project owner's instruction, 2026-11-17):** network traffic is now
allowed, but must go to the owner's own server `horizon.halcy0n.net`, never to Epic.
`compat-common::net_guard` gained a redirect mode (`TOMBOX_NET_REDIRECT=<host>`, set by
`tools/dev/iter.sh`): every requested host name is replaced by the redirect host in the
resolver calls and `WinHttpConnect`; outbound sockets are allowed only to loopback or an
address the redirect host resolves to; an entry point that cannot be trampolined fails
closed. Without the variable the guard is deny-all as before. Offline test:
`cargo test -p compat-common --test net_redirect`.

**Verified in one run:** guard logs `REDIRECT mode ... 12/12 entry points hooked`,
`'horizon.halcy0n.net' -> 4 address(es)`, five `WinHttpConnect 'datarouter.ol.epicgames.com'
-> 'horizon.halcy0n.net'` rewrites, no denied destinations, and the title logs one WinHTTP
request `State=[Succeeded]`.

**Login still fails before any login request is sent:** the GDK layer reports no signed-in
user (`LogGDKSaveGame: User -1 is not valid`, `no AuthToken stored for ControllerId -1`).
Next: find what answers the XUser calls today and what a sign-in needs.

## 2026-11-17 (late) — local Xbox user and store; login reaches the backend

**Finding.** The title runs on Microsoft's PC `xgameruntime.dll` (System32, 10.0.26100.9656).
That runtime has no user for this unregistered title, so login aborted at `ConsolePrivileges`.

**`compat-common::gdk_user` / `gdk_store` (Fortnite-build-specific, PE timestamp checked).**
The EXE's statically linked GDK wrappers (found by symbol table) are redirected: 28 `XUser*`
functions to a single local offline user (gamertag/xuid/token from `tombox.cfg`, all
privileges granted, placeholder token, no change events) and 22 `XStore*` functions to an
empty store (no updates, no products, purchases cancelled — grants nothing). Async calls
complete through the title's own XAsync wrappers.

**Result (one run):** login passes ConsolePrivileges, CheckCrossPlayPrivilege,
LoginWaitingRoom (served by the owner's backend) and LoginConsole, then
`FOnlineIdentityMcp::Login Type=xbl_xtoken` POSTs to `/account` on the backend, which answers
501 `errors.com.epicgames.authentication.grantNotImplemented`. That response is the
backend's. Xbox Live hosts (`rta.xboxlive.com`, `userstats.xboxlive.com`) are also
redirected to the backend.

**Unverified / open:** the GDK "user not found" HRESULT; other user-taking GDK families
(XGameSave, XGameUi) still reach the real runtime with the local handle and will fail there.

## 2026-11-17 (end) — login completes, game-mode select reached

After the owner's backend accepted the `xbl_xtoken` grant, the login state machine ran to
`CreatingParty --> Completed` (`HandleLoginRequestComplete Result=2`). The first attempt then
died with "GPU Crashed or D3D Device Removed": really a pipeline-create failure, because one
lobby pixel shader samples a texture with implicit derivatives inside a real loop and FXC
(SM5) cannot compile that (X3570/X3511). The translator now emits such in-loop samples as
LOD 0 (marked `APPROX-LOOP-SAMPLE`; exact only for single-mip textures) — fixture
`analysis/shaders/fortnite_loop_sample.dxbc`, 25/25 fixtures compile.

**Result (one run, driven with synthetic mouse input):** safe-zone screen, then the
game-mode select screen (Save the World / Battle Royale / Creative, "Signed in as:
TomBoxPlayer") — `analysis/evidence/2026-11-17_game_mode_select.png`.

Input note: the title reads raw mouse deltas (real PC GameInput) and accumulates them in its
3840x2160 space, so the in-game cursor moves at 1/3 window scale and is unrelated to the OS
cursor position; `tools/dev/mouse.ps1` sends relative moves. Proper window-to-title input
mapping is not implemented. No audio.

## 2026-11-17 (offline) — DXC, and every shader translating

**Compiler switch.** FXC needs minutes for some generated compute shaders (and an in-process
D3DCompile cannot be cancelled), which is what tripped the engine watchdog. DXC compiles the
same HLSL in about a second. `tombox-xshader::dxc` wraps `dxcompiler.dll` (+ `dxil.dll` for
validation/signing); `d3d12_xs` uses it when available (`TOMBOX_SHADER_COMPILER=fxc` forces
FXC). `xpso` creates a real compute pipeline state from translated DXIL plus the container's
own RTS0 root signature: 3/3 compute fixtures accepted by D3D12 on this machine.
**Not yet verified:** graphics pipelines, and the DXC path inside the running game.

**Translation coverage** (`xscan`, translate only): 105,506 of 105,506 unique shaders
(was 105,429). Added: one-of-N buffer/cbuffer descriptors (guarded copies selected at run
time); optimistic join for descriptors/table pointers known on one path only; structured
buffer addressing with OFFEN and/or IDXEN (flat addressing over the declared element size);
f16 K ops, f16 VOP3 abs/neg/clamp modifiers (previously dropped silently for VOP3-encoded
f16 ops — a correctness bug, fixed); sub-word LDS; image atomics add/umax on `<uint>` UAV
textures; wave reductions → `WaveActiveMin/Max` (SM6 only); code-table lookups whose address
passes through a VGPR; LDS vertex records indexed by output register; GS table base s22;
return from a GS-chained VS.

Inferred, not documented (see code comments): DPP operand codes 233/250, SOP1 0x29, MIMG
0x11/0x17 names, optimistic join rule.

**Compile sweep** (`xscan --compile 1 --jobs 4`, below-normal priority, fills the shared
cache): in progress. Failures found so far and fixed: high half of a 64-bit scalar move not
declared (3 shaders), position input needs centroid interpolation with conservative depth
(3 shaders).

## 2026-11-17 — PC 14.40 shader archives as a reference

The owner supplied extracted PC shader archives (14.40, `PCD3D_SM5`; kept outside the repo).
Format is the same `FSerializedShaderArchive` as the Xbox one: LZ4 entries, each =
engine resource-table header + container + engine trailer. PC containers are plain SM5 DXBC
(`ISGN OSGN SHEX`), i.e. fully documented bytecode; a few are DXIL (ray tracing).

`xpair` (exploratory) measures pairing keys that both platforms carry:

| key | Xbox entries with a PC candidate | unique candidate |
|---|---|---|
| stage + uniform-buffer layout hashes (in order) | 88,726 / 113,849 (77.9%) | 1,416 |
| + input/output semantic names | 53,668 (47.1%) | 11,743 |
| same, hashes sorted | 59,608 (52.4%) | 11,890 |

Observed: layout hashes are identical across platforms (e.g. `View`); the Xbox compiler
writes centroid inputs as `TEXCOORD10_centroid` index 0. Versions differ (14.40 vs 14.50), so
a key match is a *candidate*, not proof that two shaders come from the same source.
Not done yet: disambiguation, and comparison by execution.

**First reference check (by inspection, not yet automated).** `xpair --pairs` wrote the 3
compute pairs whose key is unique on both sides (MaterialSetupCS,
ComputeDistanceFieldNormalCS, LightGridCompactCS). For ComputeDistanceFieldNormalCS the PC
disassembly (`fxc /dumpbin`, 17 instructions) and the translation of the Xbox shader agree
instruction for instruction: same thread-ID math, same constant-buffer elements, same
texture/sampler/UAV slots and component selections, same store. One shader, read by eye —
evidence that pairing and register slots line up, not a general correctness claim.

## 2026-11-17 — Declarations compared against the PC reference; typed-buffer element types

New: `decls` module (resource declarations from a PC SM5 `SHEX` chunk, and from generated
HLSL) and `xpair --decls`, which compares the two for every pair whose key is unique on
both sides and non-trivial (4,553 pairs; the 11,7xx figure above counted Xbox entries with
one PC candidate, not uniqueness on both sides).

First run (translator as committed in 07dae65):

* 2,371 pairs agree register for register; 565 differ only in which slot a resource got
  (the two compilers assign registers independently — binding slots are **not** generally
  identical across platforms, correcting the impression from the three compute pairs).
* ~1,400 differ in whole sampler+texture units or constant buffers present on one side
  only. Read as: the pairing key does not identify the same material/permutation, or the
  versions differ. These pairs are not usable as references without a stricter key.
* Clear translator fault: ~250 pairs where the PC declares `Buffer<int>`/`Buffer<uint>`
  and the translation declared `Buffer<float4>` (mostly vertex shaders). A float-declared
  read of an integer-format view is undefined in D3D12.

Fix: `infer_tbuf_types` recovers the element type of each typed buffer from the first
typed use of loaded registers (conversions, comparisons, integer/float arithmetic, use as
a memory index; texture *sampling* coordinates count as float, fetches as integer), follows
moves and bitwise logic, stops at unconditional branches, and propagates through direct
buffer-to-buffer copies. Signedness is not reliably recoverable: integer buffers are
declared `uint4` unless a signed operation is seen; the comparison treats sint/uint as
equal.

After the fix: 2,576 pairs agree; integer/float disagreements at the same register: 7
(6 + 1), all of them in pairs whose slots are permuted (both sides have the same set of
kinds). Two intermediate versions over-inferred (233, then 139 float buffers marked
integer) because the scan ran across unconditional branches and counted sampling
coordinates as integer indices — both found through this comparison.

Checked: the 36 shaders among the first 70 pairs that now declare integer buffers compile
with DXC (vs_6_0); `xpso` accepts the LightGridCompactCS pair (all five buffers now match
the PC types). Not checked: behaviour in the game (new `d3d12_xs.dll` staged, not run).
No fixture added: the evidence needs the game archives, which stay out of the repository.

## 2026-11-17 (night) — Lobby: empty scene depth traced to swapped MUBUF opcodes

Run with the typed-buffer inference: lobby looked unchanged. Render-target capture
(`TOMBOX_RTCAP=1`) at the lobby:

* G-buffer targets contain the character and floor, correctly shaded, in the top-left
  1600x900 of the 3200x1800 targets (the title renders at 50% dynamic resolution).
* Scene depth (D32S8 3200x1800): 1 non-zero sample of 639,600. Derived-PSO depth state
  shows the base pass tests GREATER_EQUAL with depth writes **off** and a separate
  depth-only pass (no colour target) writes depth — so an empty depth buffer means the
  depth pre-pass draws nothing visible, and everything after it (fog, lighting, post)
  treats each pixel as infinitely far.

Translating the dumped depth-only vertex shader (position-only stream + primitive id →
`StructuredBuffer` of per-primitive transforms) showed impossible code: a 4-dword buffer
load into v0..v3 immediately followed by a load indexed by v3, and the same pattern with
v10..v13. The ISA only makes sense if those loads write three registers. The MUBUF opcode
table had 0x0e/0x0f (and 0x1e/0x1f for stores) the wrong way round: on this hardware
generation 0x0e is `dwordx4` and 0x0f is `dwordx3`. Evidence: destination spacing in the
same shader (0x0f loads into v13, v16, v19 — three apart; the one 0x0e load into v9 leaves
four registers before v13). The store pair was swapped by analogy with the load pair and
the public LLVM opcode order; no store has been examined.

Result (same procedure, driven to the lobby by script): depth has 160,200 non-zero samples
of 639,600 — the whole rendered quarter. The lobby now shows the backdrop, stage, slot
markers and particles (screenshot kept locally, not committed).
**Still wrong:** the image is heavily over-exposed (white character and floor, blue cast).

Tooling fixes: `mouse.ps1`/`capture.ps1`/`click.ps1` looked for a window titled exactly
"Fortnite" (the title has trailing spaces) and could not take the foreground from a
background shell; both fixed, `drive_lobby.sh` works unattended again.

## 2026-11-17 (night) — Over-exposure traced to a structured buffer read as a constant buffer

New diagnostics in `d3d12_xs` (all behind `TOMBOX_RTCAP=1` / `rtcap=1`):

* **Draw trace.** Hooks on SetPipelineState, OMSetRenderTargets, Draw*, Dispatch,
  ExecuteIndirect and on CreateRenderTargetView/CreateDepthStencilView attribute each draw of
  the captured frame to (pipeline, render targets, depth target). A capture writes
  `draws.txt` (targets by capture number, shader hashes, depth/blend state) and the original
  shader containers under `shaders/`. Recording order across command lists, not execution
  order.
* **Skip list.** `%TEMP%\tombox_skip_shaders.txt` (hashes, re-read every present) drops draws
  whose pipeline uses a listed shader — live bisection without restarting.
* **Shader override.** `%TEMP%\tombox_shader_override\<hash>.hlsl` replaces the generated HLSL
  of one shader (logged as `OVERRIDE`), to instrument a translation.

Procedure and result:

1. Scene colour in the lobby had green ≤ 1.2 but red up to ~1,400 and blue up to ~33,800.
2. Skipping each additive pass into scene colour in turn: only skipping pixel shader
   `815832c94bb46f01` (entry `ReflectionEnvironmentSkyLighting`) removed the excess.
3. Three instrumented copies of that shader (intermediate values written out in screen
   strips) showed sane G-buffer decode, specular colour, reflection-capture and sky sums; the
   excess comes from the diffuse sky term, i.e. the spherical-harmonics evaluation.
4. Its coefficients are read with `s_buffer_load_dwordx16/x8/x4` through a descriptor that
   comes from the **SRV** table (PSV0 lists a structured SRV at t0), but the translator
   mapped every scalar buffer load to a constant buffer — here `b0`, unrelated data.

Fix: a scalar load whose (single) candidate descriptor is in the SRV or UAV table is
translated as a read of a structured buffer by byte offset; the element stride is queried
at run time (`GetDimensions`) because it belongs to the view, not the shader. When several
descriptors are possible the old constant-buffer reading is kept (debt).

Result: scene colour max 0.66–0.69 on all three channels; the lobby shows the dark stage,
blue backdrop and floor pads with plausible tones. **Still wrong** (screenshot kept locally):
the character is not visible; white rectangles flicker in the upper part of the image
(bloom-chain targets contain infinities); the floor centre is very bright.

## 2026-11-17 (night) — Character missing from depth: vertex inputs are packed by *read* registers

With lighting sane, the capture showed the G-buffer with the fully textured character but a
scene depth without it, so later passes shaded those pixels as background.

The skinned depth-only vertex shader (`d15c18830a4616f9`) multiplies v8..v11 by 3 and uses
them as bone-matrix row indices, v16..v19 as weights and v24..v26 as a float vector. Its
input signature is ATTRIBUTE0,1,2,3,14,4,15,5,13 with read masks 0 for ATTRIBUTE1, 2 and 5.
The translator placed input register *r* at v[4+4r], which put the bone indices at v16. The
code only fits if the fetch shader hands over one group of four VGPRs per input register
that the shader reads, in order, skipping unread ones (ATTRIBUTE3 → v8, ATTRIBUTE14 → v12,
ATTRIBUTE4 → v16, ATTRIBUTE15 → v20, ATTRIBUTE13 → v24). A second shader with a different
skip pattern (`54ecf3051dfb9659`: only ATTRIBUTE1 and 5 unread, bone indices used from v12)
fits the same rule. Base-pass shaders read every input, which is why they worked.

Result: the character is in scene depth and appears in the lobby with correct occlusion
(screenshot kept locally). **Still wrong:** it is almost unlit (dark silhouette), a white
rectangle flickers in the upper part of the image (infinities in the bloom chain), and the
floor centre is over-bright.

## 2026-11-18 — Lobby geometry: structured-buffer element size, legacy multiply, precise positions

State after the previous entry: character in depth but unlit; deferred light volumes cover
the scene yet are rejected on the character; depth shows no floor.

Findings, each by experiment in the running title (overrides / skip list / draw trace):

1. **Structured-buffer element size is taken from the shader on PC.** The depth-only vertex
   shaders read 3 dwords per row of the per-primitive data and were given a 12-byte element;
   the buffer has 16-byte elements. D3D12 addressed `index * 12`, so the depth pre-pass used
   scrambled transforms (the floor collapsed to a line; another object covered the screen at
   "infinite" distance). Overriding one shader to declare 16 bytes made the floor appear in
   depth. The PC reference (`xpair --decls`, new stride table) shows for 801 structured
   buffers at the same register: stride 16 where the Xbox code's extent is 8, 12, 16, 20, 24
   or 28 bytes (175 of them were declared wrong), stride 64 for extent 64. Rule now:
   extent of one dword → 4-byte element; 2..7 dwords → 16 bytes, larger constant offsets
   address following elements; otherwise round up to 16. The one-dword case is needed:
   `ScatterCopyCS` reads a real 4-byte index buffer, and forcing 16 there scrambled the
   per-primitive data it uploads (three different wrong scenes before this was found).
   This is a heuristic fitted to one title — see TECHNICAL_DEBT.
2. **`v_mul_legacy_f32` / `v_mad_legacy_f32`** now follow the ISA definition (product is 0
   when either operand is 0). They were emitted as plain multiplies.
3. **`precise` on position outputs** of non-pixel stages. The base pass tests depth without
   writing and relies on matching the pre-pass exactly; without `precise` the character
   showed holes once it was in the pre-pass.
4. Vertex-buffer / draw-argument trace added to the capture tooling
   (`%TEMP%\tombox_trace_shaders.txt`): confirmed both passes bind the same streams.

Check: all 139 shaders of the lobby frame compile with DXC after these changes.

Result (screenshot kept locally): dark stage with floor pads, the character on its pad
inside a translucent blue cylinder, coloured lights at the top; depth has 160,200 non-zero
samples of 639,600 and includes floor and character; no white rectangles in this capture.
**Still wrong / unverified:** the character is very dark (direct lighting not yet examined
after these fixes); whether the scene matches the real console output is unknown — there is
no reference image of this lobby.

### Follow-up, same session: why the character is dark

Instrumenting the full-screen directional light (`a06ce28faeb9dc00`): pre-exposure ≈ 1.1,
light colour ≈ (2.9, 2.6, 2.2) — both plausible — but the shader's result is ≈ 0 on every
pixel. The light-attenuation (shadow mask) target has its blue channel at 0 on the floor
and the character and 1 elsewhere; that channel is written with MIN blending by the
per-object shadow projection shader `d2397579fc2ea0fa` (and red/green by `24915c8b11bee1f2`).
Both do percentage-closer filtering with `image_gather4_lz_o` against the D16 shadow atlas.
So the lights are being shadowed out. Not yet determined: whether the shadow atlas content
or the projection/compare is wrong. The capture tool decodes D16 targets as half floats, so
the atlas cannot be read from the current reports (fix the decoder first).

## 2026-11-18/19 — Lights were skipped: integer textures and integer outputs; binding trace

**Correction of the previous entry.** The shadow mask was not the cause of the dark
character. The binding trace (below) showed that mask belongs to a low-bias rim spotlight
behind the character, and its partial value on the floor is what its own constants give.

**Cause 1: integer textures declared as float.** The deferred light shaders test
`LightingChannelsTexture.Load(..) & LightingChannelMask`. The texture is `R16_UINT`; the
translation declared every texture `float4` and reinterpreted the float's bits, so the test
failed and the light was skipped. Fix: texel type of textures read with texel fetches (and
written with image stores) is inferred like typed buffers (`img_ty`): integer when the
loaded value is used as an integer. Bitwise logic with a small constant now counts as
integer evidence; the export tail counts as none.

**Cause 2: integer outputs converted through float.** (a) Every pixel-shader colour export
ends in `s_nop; s_nop; v_cvt_pkrtz_f16_f32 x2; exp` even for `uint4` outputs, where a
float-to-half pack of an integer gives 0 (the lighting-channel copy wrote zeros). The tail
is taken to be a default that the console replaces per render-target format — **inferred
from the shape, not observed**. The unpacked operands are now exported directly (integers
for integer elements, full floats otherwise). (b) Vertex-shader integer parameters
(`PRIMITIVE_ID`) were exported as `(uint)asfloat(bits)`, i.e. always 0, so every pixel
shader read primitive 0's data. Integer elements now pass the register bits through.
Opcode 0x2f was mislabelled `v_pack_b32_f16`; it is `v_cvt_pkrtz_f16_f32`.

**Checks.** `cargo test -p tombox-xshader`: 11 pass, including 5 new tests on hand-assembled
programs (`tests/synthetic.rs`, no game data). All 139 shaders of the lobby frame translate
and compile with DXC. `xpair --decls`: 2,607 of 4,553 paired shaders agree with the PC
declarations (was 2,538); no pair is left where the PC declares an integer texture and the
translation a float one (remaining integer differences: 3 pixel shaders with an integer
UAV buffer, 1 vertex shader with an integer buffer).

**In the game (one run, lobby).** Scene colour maximum went from 0.16 to about 5; floor and
character are lit. Not correct yet: the character is cyan with panel lines although the
base-colour target holds the right skin colours; dropping all light passes and the sky
pass leaves about half of that cyan, so part of it comes from the base pass (emissive) — possibly the
game's own "cosmetics still loading" hologram (its log warns the cosmetic loadout is not
initialised), possibly a consequence of pixel shaders now reading real primitive data.
Undetermined. Bloom still shows bright blocks along the top edge (infinite values in the
half-resolution chain).

**New diagnostics (`TOMBOX_RTCAP=1`).**
- Binding trace (`bindtrace.rs`): descriptor writes and copies are recorded by hooks on the
  real device's method table (the title also reaches the real device directly, so hooks on
  the wrapper missed views). For shaders listed in `%TEMP%\tombox_trace_shaders.txt` each
  draw/dispatch logs every bound table: constant-buffer contents, and for each view the
  resource and the view's format/stride.
- Capture: 16-bit depth is decoded as UNORM; `raw <ids>` in the trigger file also writes
  those targets at full resolution as float32.

Not done: SRVs created before a slot is rebound still show as "no descriptor recorded" in
some tables (scene depth); `R16_UINT` targets are not decoded by the capture; whole-archive
DXC sweep not rerun with these changes.

## 2026-11-20 — Vector-ALU conformance on the GPU; opcode tables checked against AMD's specs

Goal: find translator semantic bugs systematically instead of one at a time in the lobby.
Done without game data (no dump on this machine); everything below runs from the repo.

**Opcode tables vs. AMD's machine-readable ISA specs.** `tools/dev/isa_table_check.py`
compares `rdna2/mnem.rs` with GPUOpen's `amdgpu_isa_rdna1.xml` / `amdgpu_isa_rdna2.xml`.
Both references are needed: the 46 shader fixtures contain GFX10.1-only opcodes (VOP2
`v_mac_f32` ×9, `v_madak_f32` ×37; VOP3 `v_mad_f32` ×15, `v_mad_legacy_f32` ×78) next to
RDNA2 ones, so the console ISA is neither reference alone (opcode census:
`cargo run --release -p tombox-xshader --example opstat -- <shaders>`). Wrong entries found
and fixed (all unused by the fixtures, so latent for this title):
- VOPC 0xd0–0xd7 are `v_cmpx_*_u32` (write EXEC); they were decoded as `v_cmp_{u..tru}_f16`
  (write VCC) — a silent miscompile. The f16 U..TRU compares are at 0xe8–0xef. All 32-bit
  and f16 `v_cmpx_*` forms are now decoded and translated (EXEC only, see TECHNICAL_DEBT).
- FLAT 0x0e/0x0f: `load_dwordx4` / `load_dwordx3` were swapped (the same mistake as the
  MUBUF swap of 2026-11-17).
- VOP2 0x36–0x38 are `v_fmac/fmamk/fmaak_f16`, not `mac/madmk/madak`.
- VOP3 0x16b does not exist in either reference (was labelled `v_mul_lo_i32`).
After the fixes all 496 entries match (`python -I tools/dev/isa_table_check.py <xml dir>`).

**GPU conformance harness.** `hlsl::translate_valu_probe` wraps hand-assembled VALU words
in a compute shader using the same per-instruction emitter as real shaders; `gpu::Gpu` is a
minimal D3D12 compute runner (raw vtables). `tests/valu_conformance.rs` runs 207 cases
(VOP1/VOP2/VOP3/VOPC/VOP3P, modifiers, carries, compares including CMPX) over ~3,000–6,000
input vectors each (special values + random) against reference models written from the RDNA2
ISA guide (2020-11-30, downloaded from docs.amd.com) and compares on this machine's GPU.
First run: 179 pass, 23 mismatch, 5 refused. Findings:
- **Half-float rounding.** RDNA2 rounds f32→f16 to nearest even; D3D requires round-toward-
  zero for float-to-smaller-float conversion (D3D11.3 functional spec §3.2.2), and HLSL
  `f32tof16` on this GPU truncates accordingly. Every f16 result (`v_cvt_f16_f32`, f16
  arithmetic, packed f16, `v_cvt_f16_{u16,i16}`, `v_fma_mix{lo,hi}`) was 1 ulp off in about
  a third of cases. Now an integer-only round-to-nearest-even helper (`tb_f16_rne`).
- **`v_cvt_pkrtz_f16_f32`** is round-toward-zero; it used `f32tof16`, whose rounding is the
  driver's (RTZ by the D3D rule, so this GPU already matched). Now an explicit integer-only
  RTZ (`tb_f16_rtz`). *Correction of a working hypothesis:* this does not explain the bloom
  infinities — D3D also clamps out-of-range floats written to 16-bit float render targets to
  ±65504 (spec §3.2.2), the same as the console's RTZ pack.
- **`v_cmp_o/u_{f32,f16}` (+CMPX) were wrong for NaN:** `x == x` is folded to true under
  DXC's default floating-point assumptions. Now bit-level NaN tests (`tb_isnan`). Relevance:
  HLSL `isnan` guards compile to these; with the fold they never fired.
- **`v_cmp_lg` / `nlg`** used `!=` (unordered); LG is the ordered not-equal. Fixed.
- **`v_bfe_i32`** was wrong when offset + width > 32 (a shift amount wrapped); now LLVM
  AMDGPU's definition (the guide's pseudocode omits the sign extension).
- **`v_med3_f32`** with a NaN operand now follows the ISA (min3 of the three).
- **`v_cvt_i16_f16` / `v_cvt_u16_f16`** of NaN now give 0; **`v_fract_f16`** stays below 1.
- Newly translated (were refused): `v_ffbh_i32`, `v_mul_hi_i32_i24`, `v_mul_hi_u32_u24`,
  `v_alignbyte_b32`, `v_ldexp_f32`.
- `v_mad_legacy_f32` mismatches were denormal products flushed by the GPU; the harness
  accepts a flushed denormal product (D3D leaves f32 denormals to the implementation).

**Result:** 207/207 cases pass; `cargo test --release` (workspace) passes; all 43 unique
fixture shaders translate and compile with DXC (`xscan --compile 1` over the concatenated
fixtures). Also fixed: compare type inference treated `v_cmp_f_i32` as float evidence.

**Not verified:** none of this has been run in the game (no dump here). Whether the
`v_cmp_u` fix affects the lobby's bloom depends on whether its shaders use that opcode —
the next run should count it with `opstat` over the frame's shaders.

## 2026-11-20 (later) — Image and buffer conformance on the GPU; MIMG D16 / A16 / UNORM

Goal: extend the conformance harness from vector ALU to vector memory (the handoff's next
step), still without game data.

**Harness.** `gpu::Gpu::run_bound` adds descriptor heaps, textures (1D/1D-array/2D/2D-array/
3D/cube, mips; uploaded with `GetCopyableFootprints` + `CopyTextureRegion`), typed /
structured / raw buffers, samplers, null descriptors for unused slots and UAV readback.
`hlsl::translate_mem_probe` builds a compute probe whose root signature puts SRV, sampler
and UAV tables in s4/s5/s6; the probe code `s_load`s T#/S#/V# from them, so descriptors are
resolved by the same dataflow as game shaders and declared by the same code
(`resource_decls`, split out of `finish` unchanged). `tests/mem_conformance.rs`: 70 cases,
each one MIMG/MUBUF instruction over textures whose texels encode their own address, with
exact expected values (point sampling at texel centres, gathers at texel corners, integer
LODs from power-of-two gradients or quad spacing). Implicit-LOD sampling (`image_sample`,
`_b`, `_cl`) is compiled as `cs_6_6` (compute quad derivatives) and skipped on devices
without shader model 6.6. Run on an AMD Radeon RX 6600 XT (driver 32.0.21045.29003).

**Passed as translated:** `image_load` / `image_load_mip` on every dimension, all 15 DMASKs
and out-of-bounds texels; `image_sample_lz` / `_l` / `_d` (NSA) / `_lz_o` (offsets −8..7);
cube `(s, t, face)` addressing via `tb_cube_dir`; `image_sample_c_lz` (operand order and
replication); `image_gather4_lz` (channel select and texel order) and `_lz_o` with constant
and run-time offsets; `image_store` / UAV `image_load`; `buffer_load_format_x/xyz/xyzw`,
`buffer_store_format_xyzw`, structured `buffer_load_dword{,x3}` and `buffer_store_dwordx4`,
including out-of-range indices.

**Found: MIMG bits that were silently ignored.** The decoder and emitter never looked at D16
(dword1[31]), A16 (dword1[30]), UNORM (dword0[12]), TFE (dword0[16]) or LWE (dword0[17])
(positions from AMD's machine-readable RDNA2 spec; meaning from the ISA guide, Table 41).
A shader using them got wrong values without any diagnostic:
- D16 results are now f16 (round to nearest even) packed two per VGPR, for loads, samples,
  gathers and compares; D16 stores unpack halves. D16 on integer-typed images is refused.
- A16 addresses: 16-bit uints (no sampler) or f16s (with sampler) packed two per VGPR. The
  offset and z-compare dwords stay 32-bit; the bias is an f16 in its own dword; derivatives
  stay 32-bit (16-bit ones are the separate `_g16` opcodes). The bias / derivative layout is
  not in the ISA guide; it is taken from Mesa's ACO (`aco_select_nir.cpp` `visit_tex`,
  `aco_isel_helpers.cpp` `emit_pack_v1`, mesa af38c52), which emits these encodings.
- UNORM with a sampler: coordinates are divided by the view's mip-0 size (`GetDimensions`).
  Refused on cube maps.
- TFE / LWE (status dword after the data) are refused. MUBUF LDS (dword0[16]) and TFE
  (dword1[23]) are refused too.
None of the 47 committed fixture shaders sets D16, A16, TFE, LWE or MUBUF LDS (`opstat`
now tags these bits); UNORM appears only on loads/stores, where the ISA requires it. So
there is no evidence yet that the title hits these paths; the full shader archive has not
been counted.

**Gather order.** Not in the ISA guide. Mesa's ACO returns the `image_gather4` vec4 to
Vulkan/GL `textureGather` unreordered (outside a GFX8-and-older integer-cube workaround),
and Vulkan/GL use D3D's order, so the translation's direct `Gather*` mapping is right.

**Also:** DXC errors now keep every error line (validation failures used to report only
"validation errors"); `Gpu::shader_model()`.

**Result:** `cargo test --release` (workspace) passes; all 43 unique fixture shaders still
translate and compile (`xscan --compile 1`). Their HLSL is unchanged except for
declaration order, which differs between runs of the same build (see TECHNICAL_DEBT).

**Not verified:** nothing here has run in the game. The cube-array face encoding
(`slice * 8 + face` for sampled cubes, ISA §8.2.4) is not handled: `tb_cube_dir` takes
faces 0..5 only.

## 2026-11-20 — Deterministic translation

The same shader could translate to differently ordered HLSL from one run to the next: the
loop-flag declarations of `emit_body` came from iterating a `HashMap`. The compiled-shader
cache is keyed by the HLSL text, so affected shaders missed the cache and were compiled
again at every game start (6 of 43 fixtures differed between two runs). The loop list is
now sorted. `tests/fixtures.rs::fixtures_translate_deterministically` translates every
fixture nine times and requires identical output; it fails without the fix. Not measured
in game: how many of the title's shaders were affected, and the start-up time saved.

## 2026-11-20 — RDNA2 interpolation instructions

TomBox now translates `v_cvt_off_f32_i4`, which converts a signed four-bit offset to
sixteenths, and `v_lerp_u8`, which averages four pairs of bytes with independent rounding
bits. Both operations follow the [AMD RDNA 2 ISA reference, §§12.8 and 12.12](https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf).

The GPU conformance suite exercised the compact offset conversion, the extended form with
4× scaling and clamping, and packed byte averaging. All three matched the reference
bit-for-bit on 4,156 inputs each under Proton. The full vector ALU suite passed all 210
cases. Neither instruction appears in the tracked retail shader scan.

## 2026-11-20 — 16-bit integer vector comparisons and RDNA2 coverage

TomBox now decodes and translates all six relational predicates for signed and unsigned
16-bit integers, in both the VCC-writing and EXEC-writing forms. All 24 compact opcodes
were checked against LLVM's `gfx1030` assembler. Each passed 4,156 input pairs on the
host GPU, including signed boundaries and values with unrelated upper bits. The suite
also checks the extended encoding, SDWA word selection, VCC preservation, and consecutive
CMPX instructions on inactive lanes. The translator now applies SDWA selection before
evaluating a vector comparison. [AMD RDNA 2 ISA reference, §12.9](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content)

Under Proton, 263 vector ALU cases and 71 memory cases passed. The map now includes
83 previously omitted opcode-table entries and the 24 new comparisons. The existing
retail scan accounts for 272 of 528 entries. Of 251 distinct mnemonics verified by the
vector ALU suite, 129 have no retail observation and appear in amber. Each instruction's
tooltip lists its evidence. The retail scan was not rerun for this update.

## 2026-11-20 — Coin Run shader verification

`play-coins.sh --rdna2-test` executed 287 original Xbox compute shaders through
Coin Run's normal pipeline creation and dispatch on Proton. The GPU matched the CPU
reference for 1,130,005 input vectors across 251 vector and 24 scalar ALU mnemonics.
All 71 companion memory cases passed, as did the 2D, 3D, array, cube and thread-dispatch
readbacks. The ALU and memory suites directly checked 152 of the 272 entries in the
existing retail scan. [Per-instruction results](../tools/progress/coin-rdna2-coverage.tsv)

All six unsigned `s_cmpk_*` predicates initially failed inputs with the high immediate
bit set. The decoder now zero-extends the 16-bit immediate for unsigned comparisons
and sign-extends it for signed comparisons. [AMD RDNA 2 ISA reference, §12.2](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content)

## 2026-11-20 — Scalar arithmetic and condition flags

TomBox now translates `s_absdiff_i32` and `s_cmovk_i32`. Coin Run reproduced stale
scalar condition flags (SCC) on seven instructions. Signed add, subtract and add-immediate
now set SCC on overflow. Signed and unsigned minimum and maximum set SCC when the first
operand wins a strict comparison and clear it on ties. [AMD RDNA 2 ISA reference, §§12.1–12.2](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content)

Coin Run gained 26 cases for 23 scalar instructions. They check the result and flag
against CPU arithmetic when the destination overwrites a source, and check two-word
carry and borrow against 64-bit addition and subtraction. Under Proton, all 313 shader
cases passed across 1,238,061 input vectors. Direct ALU and memory checks now cover
163 of 272 retail-observed instructions, up from 152. [Per-instruction results](../tools/progress/coin-rdna2-coverage.tsv)

## 2026-11-20 — Direct GPU checks for 203 retail shader instructions

Direct GPU verification increased from 163 to 203 of 272 retail-observed instructions.
Coin Run added checks for 34 arithmetic and branch instructions. The companion memory
suite added six buffer load and store instructions, including checks that partial stores
preserve adjacent words. Under Proton, all 358 Coin Run shader cases passed across
1,424,317 input vectors, along with 77 memory cases. [Per-instruction results](../tools/progress/coin-rdna2-coverage.tsv)

GPU readback exposed five translation errors. Unsigned scalar bitfield extraction
discarded the upper width bits. Signed extraction and 64-bit scalar left shift left
the condition flag stale. The 64-bit bitfield mask lost its upper word and wrapped
offsets at bit 32. Half-precision median selected the wrong value for NaN inputs.
The register-pair probes check both halves and the scalar condition flag. The seven
branch probes check control flow with independently seeded SCC, VCC and EXEC in a
one-thread workgroup. [AMD RDNA 2 ISA reference, §§12.1, 12.5 and 12.12](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content)

## 2026-11-20 — GPU probes for all 272 retail-observed shader instructions

Coin Run increased direct reference coverage from 203 to 269 retail-observed
mnemonics. The three remaining cache controls have scoped GPU checks. Each buffer
invalidation runs between a UAV write and a reload from another wave in a 128-thread
group. Both lower to a device memory barrier on coherent compute UAVs. All three
defined instruction-prefetch modes preserve the tested register value and condition
flag. The physical cache level and fetch distance remain unmeasured.
[Per-instruction results](../tools/progress/coin-rdna2-coverage.tsv)

The compute suite passed 438 shaders over 1,757,689 input vectors. The 98 memory
cases include both settings of the atomic return flag and every word of the scalar
buffer loads. A 256-pixel graphics reference exposed errors in flat interpolation
and diagonal quad reads. It also checks geometry emission and the intermediate
value between the two interpolation steps. The graphics probe uses DXC shader
model 6.1 for per-vertex reads. The wave64 run additionally checks nonzero upper-half
lane counts. The complete suite passed on NVIDIA under Xvfb and on Radeon with the
desktop display. All 43 distinct checked-in retail shader fixtures compiled with DXC.

The retail tracker now uses the decoded names `v_fmac_f16`, `v_fmaak_f16` and
`v_permlanex16_b32` for three existing entries. Their retail occurrence counts carry
over from the earlier scan. [AMD RDNA 2 ISA reference](https://docs.amd.com/api/khub/documents/Et~wpu9g~Ffl7d9q0QZ~Og/content)

## 2026-11-20 — Retail BVH compute shaders

TomBox now translates 6 of the 52 unique compute shaders in the supplied `XboxBvhBuilder.dll`
(SHA-256 `99f803ac22c01ef329dddb1d40be67238d2a812d01e8d040b3cb5135bc23707e`).
The previous translator rejected all 52. To repeat the scan with the DLL and DXC available,
run `xscan XboxBvhBuilder.dll --mnemonics 1 --compile 1` on Windows or under Proton.
The six kernels load root SRVs, UAVs and inline
constants through one argument block. Their compute group size comes from PSV metadata.
[PSV runtime layout](https://github.com/microsoft/DirectXShaderCompiler/blob/main/include/dxc/DxilContainer/DxilPipelineStateValidation.h)

All six compiled with DXC and ran through device slot 62 under Proton. Each dispatch launched
three groups of 32 threads with 71 active inputs. CPU references derived from the retained
DXIL checked 8,192 result and guard words per shader on NVIDIA and Radeon. Five kernels
matched bit for bit. For the geometric kernel (`9c5779bf28531062`), the reference solved
the plane intersections independently with relative tolerance `2e-4` and absolute tolerance
`2e-5` on written floats. Untouched guard words matched exactly. The 43 distinct checked-in
Fortnite fixtures also compiled with DXC.

The BVH scan adds 42 named instructions to the retail observations and records 22 unresolved
opcode tags for investigation. Only named instructions enter the map. The older Fortnite
snapshot called GFX10 FLAT opcode `0x0f` `flat_load_dwordx4`; its 14 observations now have
the corrected `flat_load_dwordx3` name. Across both sources, the new translations move
`s_mul_hi_u32` and `v_mul_hi_u32` to green, raising the combined baseline from 105 to 107
of 526. The earlier 272-green map covered Fortnite alone. Seven remaining BVH shaders
stop at an integer-add wave reduction. [BVH scan counts](../tools/progress/bvh-mnemonics.txt)

Coin Run reproduces the packed-root layout using an original shader. The suite also checks
all 16 words of a scalar load, all three words of a flat load, 64-bit shifts and comparisons,
scalar bit operations, and buffer AND/OR atomics with and without returning the old value.
Direct GPU reference checks cover 306 of 314 retail-observed instructions. Three more have
limited cache probes. [GPU results](../tools/progress/coin-rdna2-coverage.tsv)

## 2026-11-20 — Integer wave sums in BVH compute shaders

Seven BVH shaders use a masked, unsigned 32-bit sum across a wave. The translator
now recognizes both the 32-lane and 64-lane sequences. It preserves the intermediate
vector and scalar registers and restores EXEC. The 64-lane form reads the upper half
from lane 32 before broadcasting the final result from lane zero. The compute shader
requests its guest wave width through shader model 6.6. Lane reads need participating
source lanes, so the translator checks for full waves and branches that could skip
the reduction. [AMD RDNA2 ISA](https://www.amd.com/content/dam/amd/en/documents/radeon-tech-docs/instruction-set-architectures/rdna2-shader-instruction-set-architecture.pdf) · [HLSL wave size](https://microsoft.github.io/DirectX-Specs/d3d/HLSL_SM_6_6_WaveSize.html)

Coin Run compared the sums with independent wrapping-integer references in 128-thread
groups with varying lane masks. Wave32 passed on NVIDIA and Radeon, and wave64 passed
on Radeon. Separate register checks cover both widths of `s_or_saveexec`, including
overlapping source and destination registers and SCC. Direct GPU checks now cover
308 of the 314 retail-observed instructions. [Coverage report](../tools/progress/coin-rdna2-coverage.tsv)

All seven shaders reached a later instruction during translation. Six next encounter
a prefix sum, and one encounters a floating-point minimum with a different prologue.
A fresh DXC scan still compiled 6 of 52 BVH shaders and all 43 distinct checked-in
Fortnite fixtures. The combined retail map remains at 107 green instructions.

## 2026-11-20 — Wave64 histogram GPU readback

The BVH histogram shader (`df7efd69ef9e6a32`) copies the low word of its root
argument pointer before masking the high word. It uses both halves of a wave64
comparison result as integers to count inputs in each bin, then combines counts
in group-shared memory with `ds_add_u32`. TomBox now recognizes those paired
ballot uses and requests a 64-lane wave through shader model 6.6.

Under Proton on Radeon, three groups of 512 threads each processed 2,048 input
words per group. Two runs used pseudorandom data with bit shifts 0 and 13 and
output strides 5 and 7. A third used a single bin per group with shift 16 and
stride 6. All 768 bin counts and all 7,424 guard words matched the integer CPU
reference in each run. Coin Run repeats the two-word ballot check using a
homebrew shader and an independently constructed lane mask.

A clean DXC scan compiled 7 of 52 BVH kernels and all 43 distinct checked-in
Fortnite fixtures. This was the last observed untranslated kernel containing
`ds_add_u32` or `s_not_b32`. Both are now green, bringing the combined retail
tracker to 109 of 526. [BVH scan counts](../tools/progress/bvh-mnemonics.txt)

## 2026-11-20 — Retail prefix scans and radix sort

Nine more kernels passed GPU readback, bringing the BVH result to 16 of 52. Seven
compute exclusive prefix sums over two, four or eight words per lane. Their
five shared-memory swizzles gather a subtotal within each 32-lane half-wave.
The 64-lane forms broadcast lane 31 to carry that subtotal into the upper half.
Three-group dispatches matched a wrapping CPU sum with block totals both enabled
and disabled, including the untouched words of both buffers. Three of the variants
combine four or eight waves per group using an LDS subtotal loop. The wave32 variants
passed on NVIDIA and Radeon. The wave64 variants passed on Radeon.

The scatter kernel uses the same swizzles, dynamic lane reads, 64-bit ballots
and shared-memory stores. Each of three Radeon dispatches matched a stable CPU
sort of 6,144 key/index pairs by the selected byte. The checks covered two
shifts on pseudorandom input and a single-bucket case. A separate kernel has 19 inline
constants before its root descriptors. Its record copy, first-record flag and
computed marker write passed on NVIDIA and Radeon with 0, 1 and 71 active records.

Coin Run includes original wave32 and wave64 prefix-sum fixtures. The optional
retail test feeds GPU histogram counts into a two-level prefix scan and then into
scatter. It checks all 16,384 key/index pairs after each of four byte passes,
including the order of equal keys and output guards. The test uses the GXDK
250405 command-list barrier between dispatches. [Run the sort](../tools/homebrew/README.md#retail-bvh-radix-sort)

A fresh DXC scan compiled 16 BVH kernels and all 43 distinct checked-in Fortnite
fixtures. The final outstanding retail uses of `ds_read2st64_b32`,
`ds_write2st64_b32`, `ds_swizzle_b32`, `s_or_b64`, `v_cmp_ne_i32` and
`v_mbcnt_hi_u32_b32` now translate. This raises
the combined tracker from 109 to 115 of 526. [BVH counts](../tools/progress/bvh-mnemonics.txt)

## 2026-11-20 — Retail FidelityFX postpasses and VOPC

The Scarlett upscaler DLL (SHA-256 `faff02f294a7e9149ce079dffe3dedbcabe99b1f180ab14222aa8689e00a75fa`)
contains 29 distinct DXBC compute shaders. Their ACCC chunks carry an FSPX v1
section directory. Section 6 holds native RDNA2 code, and PSV0 carries the
workgroup dimensions. The translator recognizes the wave32 postpass layout,
loads b0 via its root CBV and resolves the raw UAV against both the root table and
PSV binding. A wave32 comparison can write s8 while a separately computed
predicate remains live in s9.

All 13 tensor border-clear postpasses compiled with DXC. Each passed three GPU
readback cases on the RTX 4070 Ti SUPER and the Ryzen 7 9800X3D integrated GPU
under RADV. The 16 failures in the full-library scan comprise eight unsupported
compute register layouts, five vector OR reductions, two floating-point add
reductions and one lane read. These are the first reported translation errors per
shader. [Reproduce the postpass test](../tools/homebrew/README.md#retail-fidelityfx-postpasses)

The new scan makes `v_cmpx_le_i32` green with 13/13 translations. Failing FSR4
shaders also use `v_cmp_eq_i32`, `v_cmp_ne_i32` and `v_cmp_ge_u32`, putting those
three back in amber. Across the three retail sources, 9 of 122 named VOPC
instructions are green, 33 appear in shaders still failing translation, and 80
have GPU probes only. The overall retail-green count is 87/526. [FidelityFX scan counts](../tools/progress/fsr-mnemonics.txt)

## 2026-11-20 — FidelityFX prepass instruction checks

Kernel 27 is the sole scanned retail shader using `v_cmp_u_f32`. TomBox now
decodes all 693 instructions in its 4,008-byte program. Its two FP32 quad
permutations can fetch a saved register from an inactive neighbor. The prepass
also uses 64-bit LDS reads, 128-bit LDS writes, packed FP16 fused multiply-add
and float-to-byte packing. Native GFX1036 dispatches supplied reference values
for byte rounding, a fused FP16 midpoint and half-register destination writes.
The prepass exposed incorrect handling of absolute-value comparison operands
and the untouched half of a `v_fma_f16` destination.

Coin Run checked the two lane permutations and a shared-memory exchange in
which some threads exit before the group barrier. All 507 generated instruction
cases passed on the RTX 4070 Ti SUPER and on AMD using RADV, covering 2,041,192
input vectors per run. Across the three retail sources, 320 of 336 observed
instructions now have direct GPU checks. Three additional instructions have
limited cache probes. [Coin Run verification](../tools/homebrew/README.md#rdna2-verification)

Cold compilation passed for 13/29 FidelityFX shaders, 16/52 BVH shaders and
43/43 distinct Fortnite fixtures. The FidelityFX failures now start at ten
unsupported register layouts, five vector OR reductions and one lane read.
Kernel 27 stops at its 256-thread register layout. Its first table load at
`0x006c` uses byte offset `0x240`, consistent with raw SRV `t18` at a 32-byte
descriptor stride. Its shader metadata also declares constant buffer `b1`, raw
SRV `t17`, five texture SRVs, a static sampler and two UAVs. These bindings and
the full prepass output still need end-to-end verification.
The tracker now lists 530 named instructions, with 87 retail-green and 9/122
green in VOPC. [FidelityFX scan counts](../tools/progress/fsr-mnemonics.txt)

## 2026-11-20 — FSR4 prepass and 16-DLL audit

Kernel 27 now translates through the normal Xbox compute PSO path. The FSPX
binding analysis follows both descriptor tables and root CBV pairs through
forward branches. It verifies the conversion to a byte-addressed buffer before
mapping vector accesses, the PSV bindings before emitting image accesses, and
the inline sampler against the serialized static sampler.

The retail shader passed texture and quantized tensor readback for reset, mixed
NaN/finite motion, and valid history on NVIDIA and AMD RADV. The fresh scans
compiled 14/29 upscaler shaders, 16/52 BVH shaders and 43/43 distinct Fortnite
fixtures. Six instructions became retail-green, bringing VOPC to 10/122 and
the full instruction map to 93/530. [Prepass readback](../tools/homebrew/README.md#retail-fidelityfx-prepass)

Of 611 distinct static imports across the 16 DLLs, 610 resolved and
`xmem!XMemVirtualQuery` was missing. The audit records the distinction between
missing exports, exported stubs and runtime-loaded service candidates.
[Per-DLL findings](COMPATIBILITY.md#scarlett-dll-audit)

## 2026-11-20 — Matching signatures between a pixel shader and the stage feeding it

Fortnite (build 2.0.1461.7919, run offline on Windows with the network guard in deny mode)
stopped at its third graphics pipeline after `39adf8e`: `CreateGraphicsPipelineStateX`
returned `E_INVALIDARG` and the engine reported "GPU Crashed or D3D Device Removed". The
D3D12 debug layer gave `CREATEGRAPHICSPIPELINESTATE_SHADER_LINKAGE_REGISTERINDEX` (#660) and
`_REGISTERMASK` (#662) for `SV_RenderTargetArrayIndex` between a geometry and a pixel shader.
On `be622e8` (before `39adf8e`) the same setup ran to the time limit (33 pipelines, 5,400
frames, title screen).

**Cause.** DXC packs two signature elements into one register only if their interpolation
modes match. Since `39adf8e` the pixel shader declares per-vertex inputs `nointerpolation`
(`TB_VERTEX_INPUT`, read with `SV_Barycentrics`) and flat ones `nointerpolation`, while the
stage feeding it declares no modifiers. In the failing pair the geometry shader wrote
TEXCOORD0.xy, SV_Position and SV_RenderTargetArrayIndex to r0.xy, r1 and r2.x, and the pixel
shader, which reads TEXCOORD0 per vertex, had SV_RenderTargetArrayIndex in r0.z (DXC
listings of the translated HLSL). Adding `nointerpolation` to the geometry shader's
TEXCOORD0 output moved its SV_RenderTargetArrayIndex to r0.z as well.

**Fix.** A translated pixel shader reports the modifier of each input
(`Translated::ps_input_interp`). `hlsl::translate_linked` gives the outputs of the vertex or
geometry shader that feeds it the same modifiers. `d3d12_xs` now translates the pixel shader
of a pipeline first and passes its modifiers to the geometry shader, or to the vertex shader
if there is none. The modifiers are part of the runtime cache key, so a vertex shader used
with differently interpolating pixel shaders gets one translation per set. Without a pixel
shader, and for other stages, translation is unchanged. SV_Position is not mirrored.

**Diagnostics.** When D3D12 rejects a graphics pipeline built from translated shaders, the
title's original stage bytecode is saved as `%TEMP%\tombox_shaders\psofail_<n>_<stage>.dxbc`
(first ten failures), so the pair can be translated and compiled offline.

**Tests.** `tests/synthetic.rs`: a hand-assembled pixel shader reading TEXCOORD0 per vertex
with an integer TEXCOORD1, and a vertex shader exporting both. One test checks the emitted
modifiers. The other compiles both with DXC and compares the ISG1/OSG1 registers and masks,
the rule D3D12 applies. It requires the unlinked pair to mismatch (TEXCOORD1 in r0.z and
r2.x) and the linked pair to match. `cargo test --release` (workspace) passes.

**In game (one run, 180 s limit, AMD RX 6600 XT):** 33 graphics pipelines created, none
failed, 6,600 frames presented, title screen reached. The vertex-to-pixel case is covered
only by the synthetic test. A pixel shader whose inputs are a subset of the producer's
outputs can still be packed differently. That case is not handled and was not observed.

## 2026-11-21 — Optional user-language override (`locale=` in tuxbox.cfg)

A console reports the language chosen in its settings. On a host, a title sees the Windows
user's language, so on a non-English Windows a title starts in that language with no way
to change it from TomBox. Fortnite accepts `-culture=` on its command line, which is
specific to Unreal Engine.

**Change.** `compat-common::locale`: with `locale=<name>` in `tuxbox.cfg` (or the older `tombox.cfg`) the process-wide
user-language queries answer with that locale: `GetUserDefaultLocaleName`,
`GetUserDefaultLCID`, `GetUserDefaultLangID`, `GetUserDefaultUILanguage` and
`GetUserPreferredUILanguages` (both the name and the ID form, including size queries).
Without the line nothing is hooked. Windows validates and canonicalises the name
(`LocaleNameToLCID`, then `LCIDToLocaleName`, so `en-us` becomes `en-US`). An unknown name,
or one without its own LCID, is logged and changes nothing. The hooks are installed with the
other process-wide setup (`net_guard::ensure`), before the title's first query. An entry
point is patched only if the 12-byte jump stays inside the function, by its unwind data or,
for a leaf without unwind data, a `jmp rel32` thunk followed only by `int3` padding
(`GetUserDefaultLangID` on the tested host).

**Evidence.** Imports: Fortnite reads `GetUserDefaultLocaleName` and formats with
`GetLocaleInfoEx`/`LCIDToLocaleName` on the returned name; a second GDK title reads only
`GetUserDefaultLocaleName`. Neither imports `XPackageGetUserLocale`.

**Tests.** `locale::tests`: validation and canonicalisation by Windows, the buffer contract,
each hook's answer (name, LCID, LANGID, preferred-UI list in both forms, size queries,
invalid flags), and that every hooked `kernelbase` export can take the patch.
`cargo test --release -p compat-common` passes.

**In game (native Windows 11; not run under Proton).** On a German Windows with `locale=en-US`: the second title's UI is English (it
was German before), and Fortnite, started without `-culture=`, logs `Using OS detected
language (en-US)` and `locale (en-US)`. The install log lists all five entry points as
overridden.

## 2026-11-21 — `Map` of GPU-only resources (unified memory)

A second GDK title faulted right after creating its first render target: its helper for new
committed resources calls `CreateCommittedResource`, then `Map(0, NULL, &p)` without checking
the HRESULT, then `memset(p, 0, GetResourceAllocationInfo(desc).SizeInBytes)` and `Unmap`,
for every heap kind including DEFAULT-heap render targets (disassembly of the helper). On the
console every resource is CPU-visible; PC D3D12 refuses `Map` on a heap without CPU access
(`E_INVALIDARG`, debug layer `MAP_INVALIDHEAP`) and leaves `p` NULL.

**Scope.** The initial prototype returned CPU scratch memory for GPU-only resources generally.
Its author reported that the title presented on Windows with the companion texture-state
change. Reads and nonzero writes through that prototype could disagree with GPU memory.

The revised implementation enables scratch memory only for a configured executable build
and a verified pair of creation and mapping call sites. The helper must clear the entire
allocation before any GPU use. The creation and map must run on the same thread. Scratch
storage belongs to the resource through D3D12 private data and is released on unmap or
resource destruction. Other callers use the host resource's own Map and Unmap methods.

The Proton real-device regression exercises the synthetic helper on a 1904 × 1041 RGBA8
render target, checks the 0x870000-byte allocation, rejects unrelated and repeated maps,
and releases a resource with a mapping still open. An upload buffer retains a nonzero
pattern across a native unmap and remap. A separate executable loaded the release DLL,
copied 64 KiB of nonzero data through a DEFAULT buffer on the GPU and verified every byte
via a READBACK buffer. The direct map after GPU use was refused with the workaround both
enabled and disabled. The retail executable timestamp and call-site RVAs are still
needed to verify this revision on that title.

## 2026-11-21 — Buffer-only resource states on textures

A second GDK title presented three frames, then every `Present` returned
`DXGI_ERROR_DEVICE_REMOVED`. The debug layer showed the chain: `ResourceBarrier` with
`VERTEX_AND_CONSTANT_BUFFER | INDEX_BUFFER` on a Texture2D (#528,
`RESOURCE_BARRIER_INVALID_RESOURCE`), so the list failed to close (#838), and executing it
removed the device (#232, `DXGI_ERROR_INVALID_CALL`). A barrier trace named the transitions:
the title's render target goes `RENDER_TARGET <-> 0x8c3`, i.e. `GENERIC_READ` without
`INDIRECT_ARGUMENT`. The console accepts the two buffer-only bits on a texture.

**Change.** A texture cannot be read as vertex, index or constant data, so `d3d12_xs` drops
those two bits for non-buffer resources: in transition barriers (the title's barrier array is
copied only when a barrier changes) and in the initial state of `CreateCommittedResource`,
`CreatePlacedResource` and `CreatePlacedResourceX`. The remaining read states (shader
resource, copy source) keep their synchronisation. Buffers are unchanged. The first four
rewritten barriers are logged.

When the guest states become identical on PC, the wrapper omits that transition,
including each half of a split transition. The command list, placed-texture uploader,
and diagnostic capture all use the resulting host states. Capture reads subresource zero
and skips it while a split transition is pending.

**Proton verification.** The homebrew state test starts a placed texture in guest state
`0x83`, submits a split transition that disappears on PC, and then changes mip 1 to
guest state `0x803`. It checks all six uploader barrier arguments and all 1,280 pixels.
A separate capture run reported `0x8c0` for both a texture created in `0x8c3` and one
transitioned to it, and skipped a third texture with a pending split transition.

**Contributor’s retail run (native Windows 11, original PR).** No more device removal: one run presented 3,600+ frames in 90 s under the debug
layer. The remaining debug-layer errors are root CBV addresses not aligned to 256 bytes,
which this change does not address.

## 2026-11-21 — Finding GDK wrappers by interface and slot

The local offline user (`local_user=1`) was installed only into the recorded Fortnite
build, through a table of that build's wrapper addresses. A second GDK title has the same
need: the host runtime has no user for an unregistered title, the title requires
`XUserAddResult` to succeed, and it called `std::terminate` when it failed.

**Evidence.** A GDK title does not import `XUser*` or `XAsync*` by name. Each API is a small
wrapper in the title's image that asks `xgameruntime.dll` for an interface
(`QueryApiImpl(&guid, &iid, &obj)`) and calls one vtable slot. Over the Fortnite EXE, a scan
for that shape finds all 34 wrappers at exactly the addresses its symbol table names. The IID
must be part of the key: `XUserGetGamertag` uses a later version of the same object, where
slot 0x18 is not `XUserDuplicateHandle`.

**Change.** New `compat-common::gdk_wrappers`: scans the loaded image's `.pdata` functions
for that shape and keys each wrapper by `(GUID, IID, slot)`; a redirect therefore always
starts at a real function entry. `gdk_user` uses it instead of the address table. It installs
only when all six `XAsync` wrappers and `XUserAddAsync`/`XUserAddResult` are found
unambiguously. Any other wrapper that is missing or ambiguous is skipped and logged, and so
is every wrapper of these objects whose slot or IID the module does not implement.

The second title calls `XUserFindForDevice` and the device-association registrations
through the same object and slots (0x18, 0x20, 0x28) as Fortnite, but with another IID. With
exact (GUID, IID) pairs those wrappers were neither redirected nor reported, so the lookup
reached the host runtime, which does not know the local user, and the title reported its
controller as disconnected. That second IID is now accepted wherever the first one is (same
arguments at the same slots). The scan returns every wrapper of a requested object whatever
IID it passes, so an unknown interface version appears in the install log instead of
disappearing. The first four `XUserFindForDevice` calls are logged.

`local_user=1` stays opt-in. The XStore layer still uses build-specific addresses and stays
gated on the Fortnite build timestamp.

**Tests.** `gdk_wrappers::tests`: a slot with a separate IID, IID equal to GUID with a disp32
slot, the interface version as part of the key, rejection of non-wrappers, `.pdata` bounds,
and a known object's wrapper under another IID. The opt-in check over a real title,
`TOMBOX_FORTNITE_EXE=<path> cargo test --release -p compat-common --lib -- --ignored`, finds
every Fortnite wrapper at its symbol address. `cargo test --release -p compat-common` passes.

**In game (native Windows 11; not run under Proton).** Fortnite: 28/28 `XUser` wrappers
redirected, as with the address table; it reaches its title screen. One unhandled wrapper is
reported (slot 0x30 of a third IID). Second title: 22/28 redirected (the other six are not
linked into it), `XUserAddResult` succeeds and it reaches its 3D main menu. With the second
IID accepted, the title keeps its controller and the menus respond to it. One unhandled
wrapper is reported (`XUser` slot 0x68).
