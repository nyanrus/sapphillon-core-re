# Build Setup

The workspace requires a working C linker. Two options:

## Option A — VS Build Tools (MSVC, recommended)

1. Download and run the VS Build Tools installer:
   https://aka.ms/vs/17/release/vs_BuildTools.exe
2. Select **"Desktop development with C++"** workload → Install.
3. Ensure the MSVC toolchain is active:
   ```
   rustup default stable-x86_64-pc-windows-msvc
   ```
4. Open a **Developer Command Prompt** (or run `vcvars64.bat`) so that
   MSVC's `link.exe` is first in `PATH` — it must shadow Git's `link.exe`.

## Option B — MSYS2 + MinGW64 (GNU toolchain)

1. Install MSYS2: https://www.msys2.org
2. Open the MSYS2 MinGW64 shell and run:
   ```
   pacman -S mingw-w64-x86_64-gcc mingw-w64-x86_64-binutils
   ```
3. Add `C:\msys64\mingw64\bin` to your **system** `PATH`.
4. Switch Rust to the GNU toolchain:
   ```
   rustup default stable-x86_64-pc-windows-gnu
   ```

## Building

```
cargo check              # type-check all crates (fast)
cargo build              # full build
cargo build -p sapphillon_deno  # build the cdylib separately
```

## Runtime setup

Before running any workflow, load the Deno cdylib:
```rust
sapphillon_core::init_js_engine("path/to/sapphillon_deno.dll")?;
```
