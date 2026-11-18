# ADR 001: native Rust only

Date: 18 November 2026. This decision supersedes the earlier Electron fallback and the original mixed Rust/TypeScript proposal.

The owner requires the complete application to use Rust, with no JavaScript or TypeScript. Missing local tooling is resolved by installing Rust, rather than changing the implementation language. Rust 1.100.0 and the Windows GNU toolchain are installed; the project pins that compiler and a Cargo lockfile.

The UI uses native egui/eframe 0.33.3 with the OpenGL/glow backend. Repository discovery, watching, CommonMark extraction, persistence, recovery, and safe saves are Rust. Mermaid parsing/layout uses the pinned pure Rust mermaid-rs-renderer 0.3.1 library. SVG normalization/rasterization uses usvg/resvg 0.47 and native geometry uses lyon. A supervised process of the same executable owns parsing/layout/normalization and export; cancellation terminates that process. No browser, WebView, Node, npm, JavaScript runtime, WASM binding, or TypeScript is used by the native app.

Mermaid.js is a JavaScript implementation. The native Rust renderer is a different implementation, so upstream Mermaid.js exact-layout/syntax/family compatibility cannot be assumed. The owner's language constraint supersedes the original requirement to run the upstream JavaScript engine. Compatibility and unsupported cases are explicit, and source remains the source of truth.

The discarded Electron sources, node_modules, frontend output, and Electron packages were moved out of the project into a sibling backup. They are not consumed or bundled. There is one active native implementation.

The application uses UTF-8 byte spans and CommonMark source mappings. Local JSON state and recovery files are used in this implementation; SQLite remains a possible measured follow-up. Fonts are bundled with their actual OFL notices. Standard OS font discovery, when required by the native layout engine, is local rather than a remote asset fetch.

No tests or benchmarks are added or run until the owner requests them. Build success alone does not satisfy performance, fidelity, accessibility, installation, or platform release gates. This is an implementation choice under an explicit language constraint, not a claim that the original benchmark targets have passed.

Primary references: [egui](https://github.com/emilk/egui), [Rust Mermaid renderer](https://github.com/1jehuang/mermaid-rs-renderer), [resvg/usvg](https://github.com/linebender/resvg), [pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark).
