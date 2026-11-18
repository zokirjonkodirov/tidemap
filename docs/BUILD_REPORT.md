# Build report — 18 November 2026

> Historical: this report covers the discarded Electron packaging run. The current native Rust build and portable package are produced by `cargo native build` / `cargo native package`; see the [README](../README.md).

The production build and Windows x64 packaging completed successfully.

- `npm run build`: Svelte check reported 0 errors and 0 warnings; native TypeScript compilation, esbuild bundles, Vite assets, and dependency/font notices completed.
- `npm run dist`: completed the production build and NSIS packaging.
- `npx electron-builder`: completed final packaging after the executable icon/metadata and unsigned-build configuration were finalized.
- `npm run runtime:install`: installed the pinned local development runtime without launching the app.

Outputs:

- `release/MermaidLite-0.1.0-win-x64.exe`: unsigned Windows installer.
- `release/win-unpacked/MermaidLite.exe`: unpacked desktop application; keep the surrounding directory intact.
- `release/build-manifest.json`: artifact sizes and SHA-256 hashes.
- `dist/` and `dist-electron/`: production frontend and native bundles.
- `build-output.log` and `packaging-output.log`: build logs.

The packaged extraction/index worker is emitted as a physical ASAR-unpacked file. Fonts, dependency notices, and application assets are included in the production build.

No tests were added or run, and the application was not launched. No runtime, fidelity, performance, accessibility, installation, or cross-platform acceptance result is implied. The app uses the Electron fallback and JSON persistence assumptions recorded in ADR 001 and the implementation status document. Open the included `examples` folder when beginning manual validation.
