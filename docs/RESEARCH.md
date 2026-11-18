# Research notes and decision basis

Checked on **18 November 2026** using primary project documentation. These references support library/platform facts. Product choices and speed targets in this package are proposed engineering decisions, not claims made by the libraries. Recheck exact versions when implementation begins.

## Desktop runtime

- [Tauri process model](https://v2.tauri.app/concept/process-model/): the Rust core and OS-specific WebViews have distinct responsibilities; the engine differs across Windows/macOS/Linux. This supports a native core but creates a cross-engine validation obligation.
- [Tauri WebView versions](https://tauri.app/reference/webview-versions/): runtime availability/version is a deployment prerequisite. A small application archive does not include every possible runtime prerequisite.
- [Electron process model](https://www.electronjs.org/docs/latest/tutorial/process-model): a documented alternate process architecture if Tauri's separately supervised helper fails the feasibility gate. No blanket claim that either runtime is faster.
- [Tauri WebDriver testing](https://v2.tauri.app/develop/tests/webdriver/): current WebdriverIO service supports an embedded route across all three OS families; directly driving the original `tauri-driver` has different macOS limitations. Pin and prove the actual route used.

## Mermaid integration

- [Usage/API](https://mermaid.js.org/config/usage.html): rendering produces SVG; validation reports diagram type, not a stable general graph AST. Current v12 documentation targets ES2024 and Safari 17.4 or later. Confirm OS/runtime compatibility for the selected pinned release.
- [Layouts](https://mermaid.js.org/config/layouts): the checked documentation says ELK is bundled/default for applicable diagram families starting in v12. Older separate-package/Dagre-default advice must not be applied without checking the version. Layout availability differs by family.
- [Configuration schema](https://mermaid.js.org/config/schema-docs/config.html): finite text/edge limits, security levels, root `htmlLabels`, font and theme settings exist. The app should apply its own bounded policy and protect security options from diagram overrides.
- [Theming](https://mermaid.js.org/config/theming.html): diagram and application configuration can affect presentation. Evaluate the pinned build's theme/look support before promising exact geometry; retain authored semantics.

## Large diagrams

- [elkjs](https://github.com/kieler/elkjs): layout can use a Web Worker and exposes layout options. This does not prove that every stage of Mermaid rendering is worker-safe or that all graph layouts are fast.
- [usvg](https://raw.githubusercontent.com/linebender/resvg/main/crates/usvg/README.md): simplifies supported SVG features and resolves text/markers/styles, but unsupported features can be ignored and CSS support is limited. Therefore use explicit feature validation and differential tests. Disable external-resource resolution.
- [resvg](https://github.com/linebender/resvg): candidate SVG rasterization tool for bounded fallback/export; library availability does not prove scalable tile generation or full Mermaid fidelity.
- [CodeMirror examples](https://codemirror.net/examples/): the upstream examples include huge-document editing. This is a useful editor candidate, not evidence that Mermaid parsing or the full app will handle the same source sizes without profiling.

## Markdown

- [CommonMark fenced code blocks](https://spec.commonmark.org/0.31.2/#fenced-code-blocks): use a real parser and mapped source spans. Container indentation, fence delimiters and code content make naïve regular-expression extraction/patching unsafe.

## Fonts

- [Google Fonts Manrope metadata](https://github.com/google/fonts/blob/main/ofl/manrope/METADATA.pb) and [OFL build repository](https://github.com/googlefonts/manrope): proposed UI font distribution. Pin the exact OFL asset and include its license.
- [Current official Manrope distribution](https://www.sharanda.com/manrope): newer named builds have their own terms. Do not infer identical licenses from the font name. The proposed design uses the explicitly OFL build instead.
- [IBM Plex](https://github.com/IBM/plex): proposed source font family; include the OFL notices for bundled assets.

## Questions to resolve with evidence in P0

1. Can a packaged Tauri helper render with real font/DOM metrics and be killed/restarted independently on every declared platform?
2. Which exact Mermaid release and WebView floors meet syntax coverage, huge layout and memory needs?
3. Which SVG constructs appear in the complete bundled-family corpus, and can normalization/canvas retain them faithfully?
4. Does total huge latency come from layout, SVG construction, normalization, transfer, path preparation or paint?
5. Which thresholds keep main-window work bounded without needlessly rasterizing ordinary diagrams?
6. Is a narrow structural flowchart adapter necessary for the huge release gate?
7. Can the chosen native testing route and signing/packaging process be reproduced on clean machines?

Record answers, measurements and limitations in short architecture decisions. Research documentation is not a substitute for these experiments.
