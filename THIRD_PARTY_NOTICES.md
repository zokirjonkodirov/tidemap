# Third-party notices

Updated 18 November 2026. The native Rust application uses open-source Rust libraries. Exact resolved versions are recorded in Cargo.lock. Their licenses and notices remain applicable to bundled native code.

Direct dependencies of the workspace crates:

- egui/eframe (0.33.3) and related egui crates: MIT OR Apache-2.0.
- ttf-parser (diagram engine, text measurement): MIT OR Apache-2.0.
- usvg and resvg (0.47.0), tiny-skia: PNG export only. See their crate license notices and Cargo metadata (not re-checked for this update).
- pulldown-cmark: MIT.
- ignore, notify, directories, tempfile, rfd, image, base64, serde, serde_json, sha2, anyhow, libc (Unix only), and other native dependencies: see resolved Cargo metadata and full package license notices.
- roxmltree and xmlwriter are present only as transitive dependencies of usvg/resvg.
- Inter (Regular, Medium, SemiBold): Copyright The Inter Project Authors (https://github.com/rsms/inter); SIL Open Font License 1.1. Full license: assets/fonts/Inter-OFL.txt.
- IBM Plex Mono: Copyright IBM; SIL Open Font License 1.1. Full license: assets/fonts/IBMPlexMono-OFL.txt.

Removed on 18 November 2026 with the move to the in-house diagram engine: mermaid-rs-renderer, lyon, bincode, and the Manrope font. They are no longer dependencies or bundled assets.

Font files are copied from the upstream OFL distributions and embedded locally. No font service is used. The Rust app does not distribute Electron, Chromium, Mermaid.js, Svelte, CodeMirror, JavaScript, or TypeScript.
