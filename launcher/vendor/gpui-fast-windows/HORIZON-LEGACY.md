# gpui-fast-windows 0.1.13, modified for Horizon Legacy

Copied from crates.io (Apache-2.0, see LICENSE). The one change is in `src/directx_renderer.rs`,
`mod shader_resources`: the shaders are compiled when the renderer starts, from the HLSL embedded in the
binary, by `D3DCompile` (Windows' own `d3dcompiler_47.dll`), instead of by `fxc.exe` at build time. A
build on Linux for `x86_64-pc-windows-gnu` has no `fxc.exe`, so without this release builds fail on
`shaders_bytes.rs`.
