# Run TuxBox on Linux or Windows

## Linux with Proton

From the repository root, run `./tuxbox test` for the native analysis tools. For the first graphics check, build Coin Run with `tools/homebrew/build-clear.sh` and run `./tuxbox homebrew --3d-test`. The homebrew suite keeps its existing prefix under `target/x86_64-pc-windows-msvc/release/coin-prefix`. [Dependencies and test modes](../tools/homebrew/README.md)

Launch a prepared executable from Linux with the same app-local DLLs, config and assets that it expects on Windows. For example, after building Coin Run:

```sh
./tuxbox run target/x86_64-pc-windows-msvc/release/clear_pixel.exe --triangle
```

The launcher searches the Steam installation and its registered libraries for Proton Experimental. To select an executable yourself, set `PROTON`. Use `STEAM_COMPAT_CLIENT_INSTALL_PATH` for a custom Steam root. For `run`, the default persistent prefix is under `$XDG_DATA_HOME/tuxbox/proton-prefix` (normally `~/.local/share/tuxbox/proton-prefix`). Set `STEAM_COMPAT_DATA_PATH` to select an existing prefix. Prepare any title-specific drive mappings and dependencies in the selected prefix. Retail Fortnite startup on Proton remains unverified.

For a title that needs Microsoft’s UCRT, place the x64 Windows SDK redistributable `ucrtbase.dll` beside the prepared executable and run `./tuxbox run --native-ucrt /path/to/game.exe`. This opts that launch into Proton’s native-first UCRT selection. The runtime probe verified the loaded provider path with SDK DLL version 10.0.26100.9457. [Microsoft UCRT deployment](https://learn.microsoft.com/en-us/cpp/windows/universal-crt-deployment) · [Import coverage](COMPATIBILITY.md#scarlett-dll-audit)

## Native Windows

### Requirements

- Windows 10/11 x64 with a Direct3D 12 GPU.
- Microsoft Gaming Services (provides `xgameruntime.dll`).
- Rust toolchain (`x86_64-pc-windows-msvc`) and the Windows SDK (`dxcompiler.dll` and
  `dxil.dll`, found automatically or through `TOMBOX_DXC_DIR`).
- Your own authorised, decrypted title dump.

### Build

```
cargo build --release
cargo test --release
```

The compatibility DLLs must be built with a static CRT. Run these commands in Command Prompt.

```bat
set RUSTFLAGS=-C target-feature=+crt-static
cargo build --release -p d3d12_xs -p xmem -p xcrdapi -p acphal -p dxcompiler_xs -p mfplat
```

### Run a prepared title

Place the compatibility DLLs and dependencies required by the executable beside it, keep its asset layout intact, and launch the executable on Windows. Use its actual PE imports and package manifest to select dependencies. Additional setup varies by title.

### Windows case study · Fortnite

The steps below apply to build 2.0.1461.7919. The development scripts use machine-specific paths. [Development tools](../tools/dev/README.md)

1. Extract the dump to `input/extracted/` (game files end up under `input/extracted/Mount/`).
2. Map that folder to drive `G:`, where the game expects its filesystem root:
   `subst G: "%CD%\input\extracted\Mount"`
3. Copy `acphal.dll`, `d3d12_xs.dll`, `dxcompiler_xs.dll`, `mfplat.dll`, `xcrdapi.dll` and
   `xmem.dll` from `target/release/` into the folder of the game executable
   (`G:\FortniteGame\Binaries\XSX\`).
4. Copy the **real** `xg_xs.dll` from the dump (`EmbeddedXvd/Windows/System32/`) into the
   same folder. Texture layouts depend on it; do not use the stub crate of that name.
5. To reproduce the recorded build’s offline user and store setup, create `tuxbox.cfg` beside its executable with the following line. The user layer finds the title’s GDK wrappers by interface and slot; the store layer also checks the build timestamp. Ordinary titles leave it disabled.

   ```ini
   local_user=1
   ```

   Optional keys on separate lines are `gamertag`, `xuid`, `xbl_token`, and `net_redirect`. Set the last key only for your own test server.

   `locale=<name>` (for example `locale=en-US`) works for any title, without `local_user`: it sets the user language the title sees. Omit it to keep the host's language.
6. Run the game executable (`FortniteClient-XSX-Shipping.exe`).

The runtime reads the existing `tombox.cfg` when `tuxbox.cfg` is absent. Existing `TOMBOX_*` environment variables and diagnostic paths work on both hosts.

Compatibility-layer calls are logged to `%TEMP%\tombox_compat.log` (or `%TOMBOX_LOG%`). The
game logs only through the debugger channel; `tools/dev/mdbg.rs` is a small tracer for it.

| Variable | Effect |
|---|---|
| `TOMBOX_NET_REDIRECT` | Same as `net_redirect` |
| `TOMBOX_ALLOW_NETWORK=1` | Turn the network guard off (unrestricted access) |
| `TOMBOX_D3D12_DEBUG=1` | Direct3D validation layer |
| `TOMBOX_RTCAP=1` | Diagnostics: render-target capture, per-draw trace, shader skip list and override, binding trace (see `docs/DEVELOPMENT_LOG.md`) |
| `TOMBOX_OWN_WINDOW=1` | Present into a separate window |

### GPU resource zero initialization

For a verified game helper that immediately clears a new committed GPU-only resource
before submitting it to the GPU, add the following line to `tuxbox.cfg` beside the game
executable. Omit it for ordinary execution.

```ini
zero_init_map=<PE timestamp>:<CreateCommittedResource return RVA>:<Map return RVA>
```

All three fields are hexadecimal. Each RVA identifies the instruction immediately after
the corresponding call in the executable. The helper must map subresource zero with a
null read range, write only zeros, then unmap. Only the configured paths in that build
can use the workaround. General GPU resource readback and upload remain open work.
