# Diagram Engine Core Implementation Plan (plan 1 of 3)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A new pure-Rust crate that turns Mermaid flowchart source into a positioned, typed display list, fast and with exact source spans.

**Architecture:** `crates/diagram` has four stages with plain-data hand-offs: `parse` (&str → `Graph` + diagnostics), `layout` (`Graph` → `Layout`, layered/Sugiyama with compound subgraphs), `scene::build` (`Layout` → `Scene` display list with a grid index). No egui, no SVG, no I/O. The app is not touched in this plan; plan 2 wires it in.

**Tech Stack:** Rust 1.88 edition 2024, `ttf-parser` for font advances, `serde` for the metrics struct. Nothing else.

**Spec:** `docs/superpowers/specs/2026-11-18-own-engine-and-shell-redesign-design.md` (§5.1–5.5, §5.7 first row, §9).

## Global Constraints

- Crate name `mermaidlite-diagram`, path `crates/diagram`, added to workspace `members` and `[workspace.dependencies]`.
- No dependency on egui, usvg, resvg, lyon, or `mermaid-rs-renderer`.
- All spans are UTF-8 byte offsets into the source passed to `parse` (`u32`).
- No `unsafe`. No panics on any input: every public function returns a value or an error for arbitrary bytes.
- Every loop in `layout` that is not O(1) per element checks `cancel: &AtomicBool` at least once per outer iteration and returns `LayoutError::Canceled`.
- Hard caps: 20,000 nodes, 20,000 edges, world bounds 10,000,000 units per side → `LayoutError::Budget(String)`.
- Default metrics (spec §7): font size 14, line height 20, max label width 220, node padding x 18 / y 12, corner radius 12, rank spacing 64, node spacing 40, edge label font 13, cluster padding 24, cluster title height 28.
- Test command for everything in this plan: `cargo test -p mermaidlite-diagram`. Adding a dependency requires one `cargo build -p mermaidlite-diagram` without `--locked` to update `Cargo.lock`.
- The folder is not a git repository. Task 1 step 1 asks the owner whether to run `git init`. If yes, each task ends with a commit `feat(diagram): <task title>`; if no, skip commit steps.
- Fonts: tests load `assets/fonts/Inter-Regular.ttf` if present, else `assets/fonts/Manrope.ttf` (Inter arrives in plan 3; do not block on it).

## Review Focus

1. **Duplicate and re-declared nodes** (`A[One]` then later `A[Two]` or bare `A`): one node, last explicit label and shape win, span is the first declaration. Test in Task 3.
2. **CRLF files and non-ASCII labels** (`A[Заказ 🚀]\r\n`): spans stay valid byte ranges on char boundaries; line numbers in diagnostics count `\n` only. Test in Task 4.
3. **Edges that target a subgraph id** (`A --> Workspace`): must connect to the container, not create a stray node named like the subgraph. Test in Tasks 4 and 11.
4. **Self-loops, two-node cycles and parallel edges** (`A-->A`, `A-->B`, `B-->A`, `A-->B` twice): all drawn, none overlapping exactly, arrows keep authored direction. Test in Tasks 7 and 10.
5. **Empty or header-only input** (`""`, `flowchart LR`, only comments): empty graph, empty scene with zero-size bounds, no diagnostic, no panic. Test in Tasks 4 and 14.

---

## File structure

```text
crates/diagram/
  Cargo.toml
  src/lib.rs            public re-exports, `render()` convenience
  src/graph.rs          ids, Span, Graph, Node, Edge, Cluster, enums, Diagnostic
  src/lexer.rs          byte lexer → tokens with spans
  src/parser.rs         flowchart statements → Graph, error recovery
  src/text.rs           Measurer, wrap, TextBlock
  src/shape.rs          size, outline, boundary intersection per ShapeKind
  src/metrics.rs        Metrics (geometry knobs, serde)
  src/layout/mod.rs     layout() driver, Layout, LayoutError, internal LGraph
  src/layout/acyclic.rs cycle removal
  src/layout/rank.rs    ranking, dummy-node insertion
  src/layout/order.rs   crossing minimisation
  src/layout/position.rs Brandes–Köpf coordinates
  src/layout/compound.rs cluster borders, nesting, recursive direction
  src/layout/route.rs   ports, splines, clipping, self-loops
  src/layout/pack.rs    component packing, width snapping
  src/scene.rs          Scene, items, grid index, hit, neighbours
  tests/fixtures/*.mmd  corpus
  tests/invariants.rs   layout invariants over the corpus
  tests/never_panic.rs  byte-mutation loop
tools/xtask/src/bench.rs  `cargo native bench` timing harness
```

---

### Task 1: Crate scaffold and graph model

**Files:**
- Create: `crates/diagram/Cargo.toml`, `crates/diagram/src/lib.rs`, `crates/diagram/src/graph.rs`
- Modify: `Cargo.toml` (workspace `members`, `[workspace.dependencies] mermaidlite-diagram = { path = "crates/diagram" }`)

**Interfaces — Produces:**

```rust
// graph.rs
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)] pub struct Span { pub start: u32, pub end: u32 }
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)] pub struct NodeId(pub u32);
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)] pub struct EdgeId(pub u32);
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)] pub struct ClusterId(pub u32);
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Family { Flowchart, State, Sequence, Class, Er, Other }
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)] pub enum Direction { #[default] TB, BT, LR, RL }
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ShapeKind { #[default] Rect, Rounded, Stadium, Subroutine, Cylinder, Circle, DoubleCircle,
    Diamond, Hexagon, Parallelogram, ParallelogramAlt, Trapezoid, TrapezoidAlt, Asymmetric }
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)] pub enum LineKind { #[default] Solid, Dotted, Thick, Invisible }
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)] pub enum ArrowKind { None, #[default] Arrow, Circle, Cross }
#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum Endpoint { Node(NodeId), Cluster(ClusterId) }
#[derive(Clone, Debug, Default)] pub struct StyleProps { pub fill: Option<[u8;4]>, pub stroke: Option<[u8;4]>,
    pub color: Option<[u8;4]>, pub stroke_width: Option<f32>, pub dashed: bool }
pub struct Node { pub key: String, pub label: String, pub shape: ShapeKind, pub cluster: Option<ClusterId>,
    pub classes: Vec<u16>, pub style: StyleProps, pub span: Span, pub label_span: Span }
pub struct Edge { pub from: Endpoint, pub to: Endpoint, pub label: Option<String>, pub line: LineKind,
    pub head: ArrowKind, pub tail: ArrowKind, pub min_len: u8, pub style: StyleProps, pub span: Span }
pub struct Cluster { pub key: String, pub title: String, pub parent: Option<ClusterId>,
    pub direction: Option<Direction>, pub classes: Vec<u16>, pub style: StyleProps, pub span: Span }
pub struct ClassDef { pub name: String, pub props: StyleProps }
#[derive(Default)] pub struct Graph { pub direction: Direction, pub nodes: Vec<Node>, pub edges: Vec<Edge>,
    pub clusters: Vec<Cluster>, pub class_defs: Vec<ClassDef> }
pub struct Diagnostic { pub line: u32, pub column: u32, pub span: Span, pub message: String }
pub struct Parsed { pub family: Family, pub graph: Graph, pub diagnostics: Vec<Diagnostic> }
impl Graph { pub fn out_edges(&self, n: NodeId) -> impl Iterator<Item = EdgeId> + '_;   // linear scan is fine here;
              pub fn in_edges(&self, n: NodeId) -> impl Iterator<Item = EdgeId> + '_; } // Scene owns the fast CSR adjacency
// lib.rs
pub fn detect(source: &str) -> Family;
```

`detect` skips a leading `---` frontmatter block, `%%` comment lines and `%%{…}%%` directives, then matches the first word: `flowchart`|`graph` → Flowchart, `stateDiagram`|`stateDiagram-v2` → State, `sequenceDiagram` → Sequence, `classDiagram` → Class, `erDiagram` → Er, anything else → Other.

- [ ] **Step 1:** Ask the owner: "Run `git init` here so each task is a commit?" Record the answer at the top of this file.
- [ ] **Step 2: Write failing tests** in `lib.rs`:

```rust
#[test] fn detects_family_past_frontmatter_and_comments() {
    assert_eq!(detect("---\ntitle: x\n---\n%% note\n%%{init: {}}%%\n  flowchart LR\n A-->B"), Family::Flowchart);
    assert_eq!(detect("graph TD"), Family::Flowchart);
    assert_eq!(detect("sequenceDiagram\n"), Family::Sequence);
    assert_eq!(detect("stateDiagram-v2"), Family::State);
    assert_eq!(detect("pie title x"), Family::Other);
    assert_eq!(detect(""), Family::Other);
}
```

- [ ] **Step 3:** Run `cargo test -p mermaidlite-diagram`. Expected: fails to compile (`detect` missing).
- [ ] **Step 4:** Implement `graph.rs` types and `detect`.
- [ ] **Step 5:** Run tests. Expected: 1 passed.
- [ ] **Step 6:** Commit.

---

### Task 2: Lexer

**Files:** Create `crates/diagram/src/lexer.rs`.

**Interfaces — Produces:**

```rust
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tok { Ident, Text, Open(ShapeKind), Close, Link, Pipe, Amp, Colon3, Semi, Newline, At, LBrace, RBrace, Eof }
pub struct Token { pub kind: Tok, pub span: Span }
pub struct LinkInfo { pub line: LineKind, pub head: ArrowKind, pub tail: ArrowKind, pub len: u8, pub inline_label: Option<Span> }
pub struct Lexer<'a> { /* src: &'a [u8], pos */ }
impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self;
    pub fn next(&mut self) -> Token;                    // never panics; unknown byte → Tok::Text of one char
    pub fn link_info(&self, t: &Token) -> LinkInfo;     // decode a Tok::Link span
    pub fn shape_text(&mut self, open: ShapeKind) -> (Span, bool); // label span up to the matching closer; bool = closed
}
```

Rules the tests pin: `%%` to end of line is skipped; `;` and `\n` both end a statement; link grammar is `<`? or `o`/`x`? then a run of `-`, `=`, `.`, `~` then `>`/`o`/`x`? ; `len` = extra dashes beyond the minimum (`--->` is 1, `---->` is 2, `-..->` is 1, `====>` is 1); `-- text -->` and `== text ==>` and `-. text .->` set `inline_label`; openers are matched longest-first: `(((`, `((`, `([`, `[[`, `[(`, `[/`, `[\`, `{{`, `>`, `[`, `(`, `{`; a quoted label `["a ] b"]` ignores brackets inside the quotes.

- [ ] **Step 1: Write failing tests**

```rust
fn kinds(s: &str) -> Vec<Tok> { let mut l = Lexer::new(s); let mut v = vec![]; loop { let t = l.next(); if t.kind == Tok::Eof { break } v.push(t.kind) } v }
#[test] fn links_decode() {
    for (src, line, head, tail, len) in [
        ("-->", LineKind::Solid, ArrowKind::Arrow, ArrowKind::None, 0), ("---", LineKind::Solid, ArrowKind::None, ArrowKind::None, 0),
        ("--->", LineKind::Solid, ArrowKind::Arrow, ArrowKind::None, 1), ("-.->", LineKind::Dotted, ArrowKind::Arrow, ArrowKind::None, 0),
        ("-..->", LineKind::Dotted, ArrowKind::Arrow, ArrowKind::None, 1), ("==>", LineKind::Thick, ArrowKind::Arrow, ArrowKind::None, 0),
        ("<-->", LineKind::Solid, ArrowKind::Arrow, ArrowKind::Arrow, 0), ("--o", LineKind::Solid, ArrowKind::Circle, ArrowKind::None, 0),
        ("x--x", LineKind::Solid, ArrowKind::Cross, ArrowKind::Cross, 0), ("~~~", LineKind::Invisible, ArrowKind::None, ArrowKind::None, 0),
    ] { let mut l = Lexer::new(src); let t = l.next(); assert_eq!(t.kind, Tok::Link, "{src}");
        let i = l.link_info(&t); assert_eq!((i.line, i.head, i.tail, i.len), (line, head, tail, len), "{src}"); }
}
#[test] fn inline_label_span() { let s = "A -- yes, go --> B"; let mut l = Lexer::new(s); l.next(); let t = l.next();
    let i = l.link_info(&t); let sp = i.inline_label.unwrap(); assert_eq!(&s[sp.start as usize..sp.end as usize], "yes, go"); }
#[test] fn openers_longest_first() { assert_eq!(kinds("A((("), vec![Tok::Ident, Tok::Open(ShapeKind::DoubleCircle)]);
    assert_eq!(kinds("A([")[1], Tok::Open(ShapeKind::Stadium)); assert_eq!(kinds("A[(")[1], Tok::Open(ShapeKind::Cylinder));
    assert_eq!(kinds("A{{")[1], Tok::Open(ShapeKind::Hexagon)); assert_eq!(kinds("A>")[1], Tok::Open(ShapeKind::Asymmetric)); }
#[test] fn quoted_label_keeps_brackets() { let s = r#"A["a ] b"] --> B"#; let mut l = Lexer::new(s); l.next(); l.next();
    let (sp, closed) = l.shape_text(ShapeKind::Rect); assert!(closed); assert_eq!(&s[sp.start as usize..sp.end as usize], "a ] b"); }
#[test] fn comments_and_separators() { assert_eq!(kinds("A %% hi\nB;C"), vec![Tok::Ident, Tok::Newline, Tok::Ident, Tok::Semi, Tok::Ident]); }
#[test] fn ident_with_dash_before_link() { let s = "api-gw-->db"; let mut l = Lexer::new(s); let a = l.next();
    assert_eq!(&s[a.span.start as usize..a.span.end as usize], "api-gw"); assert_eq!(l.next().kind, Tok::Link); }
```

- [ ] **Step 2:** Run. Expected: compile failure.
- [ ] **Step 3:** Implement the lexer over `&[u8]`, advancing only on UTF-8 char boundaries.
- [ ] **Step 4:** Run. Expected: all lexer tests pass.
- [ ] **Step 5:** Commit.

---

### Task 3: Parser — nodes, shapes, edges, chains

**Files:** Create `crates/diagram/src/parser.rs`; modify `lib.rs`.

**Interfaces — Produces:** `pub fn parse(source: &str) -> Parsed` (re-exported from `lib.rs`). For a non-flowchart family it returns an empty graph, the detected `family`, and no diagnostics; the caller decides what to show.

Label text rules: surrounding quotes removed; `<br>`, `<br/>`, `<br />` and literal `\n` become `'\n'`; other HTML tags stripped; `&amp; &lt; &gt; &quot; #35;`-style entities decoded; markdown backtick strings (`` "`**bold** text`" ``) keep text, drop markers.

- [ ] **Step 1: Write failing tests**

```rust
fn g(s: &str) -> Graph { let p = parse(s); assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()); p.graph }
fn node<'a>(g: &'a Graph, key: &str) -> &'a Node { g.nodes.iter().find(|n| n.key == key).unwrap() }
#[test] fn header_direction() { assert_eq!(g("flowchart LR\nA").direction, Direction::LR); assert_eq!(g("graph TD\nA").direction, Direction::TB); assert_eq!(g("flowchart\nA").direction, Direction::TB); }
#[test] fn every_bracket_shape() {
    let src = "flowchart TB\na[r]\nb(ro)\nc([st])\nd[[sub]]\ne[(cyl)]\nf((ci))\ng(((dc)))\nh{di}\ni{{hex}}\nj[/par/]\nk[\\alt\\]\nl[/tr\\]\nm[\\tra/]\nn>asym]";
    let gr = g(src); use ShapeKind::*;
    let want = [Rect, Rounded, Stadium, Subroutine, Cylinder, Circle, DoubleCircle, Diamond, Hexagon, Parallelogram, ParallelogramAlt, Trapezoid, TrapezoidAlt, Asymmetric];
    assert_eq!(gr.nodes.iter().map(|n| n.shape).collect::<Vec<_>>(), want);
    assert_eq!(node(&gr, "c").label, "st");
}
#[test] fn at_shape_syntax() { let gr = g("flowchart TB\nA@{ shape: diamond, label: \"Ok?\" }"); assert_eq!(node(&gr, "A").shape, ShapeKind::Diamond); assert_eq!(node(&gr, "A").label, "Ok?"); }
#[test] fn bare_node_label_is_key() { assert_eq!(node(&g("flowchart TB\nAlpha"), "Alpha").label, "Alpha"); }
#[test] fn chain_and_fanout() { let gr = g("flowchart LR\nA --> B & C --> D");
    let pairs: Vec<_> = gr.edges.iter().map(|e| match (e.from, e.to) { (Endpoint::Node(a), Endpoint::Node(b)) => (gr.nodes[a.0 as usize].key.clone(), gr.nodes[b.0 as usize].key.clone()), _ => unreachable!() }).collect();
    assert_eq!(pairs, [("A","B"),("A","C"),("B","D"),("C","D")].map(|(a,b)| (a.to_string(), b.to_string()))); }
#[test] fn edge_labels_both_forms() { let gr = g("flowchart LR\nA -->|yes| B\nA -- no --> C"); assert_eq!(gr.edges[0].label.as_deref(), Some("yes")); assert_eq!(gr.edges[1].label.as_deref(), Some("no")); }
#[test] fn edge_length_sets_min_len() { let gr = g("flowchart LR\nA ---> B\nA --> C"); assert_eq!(gr.edges[0].min_len, 2); assert_eq!(gr.edges[1].min_len, 1); }
#[test] fn redeclared_node_merges_last_label_wins_first_span() { let s = "flowchart LR\nA[One] --> B\nA[Two]\nA --> C"; let gr = g(s);
    assert_eq!(gr.nodes.iter().filter(|n| n.key == "A").count(), 1); let a = node(&gr, "A"); assert_eq!(a.label, "Two"); assert_eq!(a.span.start as usize, s.find("A[One]").unwrap()); }
#[test] fn label_text_cleanup() { let gr = g("flowchart LR\nA[\"line one<br/>line &amp; two\"]\nB[\"`**bold** text`\"]"); assert_eq!(node(&gr, "A").label, "line one\nline & two"); assert_eq!(node(&gr, "B").label, "bold text"); }
#[test] fn spans_point_at_source() { let s = "flowchart LR\n  Start[Go now] --> End"; let gr = g(s); let n = node(&gr, "Start");
    assert_eq!(&s[n.span.start as usize..n.span.end as usize], "Start[Go now]"); assert_eq!(&s[n.label_span.start as usize..n.label_span.end as usize], "Go now");
    let e = &gr.edges[0]; assert_eq!(&s[e.span.start as usize..e.span.end as usize], "Start[Go now] --> End"); }
```

- [ ] **Step 2:** Run. Expected: compile failure.
- [ ] **Step 3:** Implement statement-level recursive descent: `statement := node_group (link node_group)*`, `node_group := node ('&' node)*`. Node lookup by key through a `HashMap<&str, NodeId>`.
- [ ] **Step 4:** Run. Expected: pass.
- [ ] **Step 5:** Commit.

---

### Task 4: Parser — subgraphs, styling, directives, recovery

**Files:** Modify `parser.rs`; create `crates/diagram/tests/never_panic.rs`.

Decisions: `subgraph id [Title]` / `subgraph Title` / `subgraph "Title"`; title defaults to id; nested `direction XX` sets `Cluster::direction`; a node first mentioned inside a subgraph belongs to it, later mentions elsewhere do not move it. An identifier used as an edge endpoint that equals a subgraph key (declared anywhere in the file, before or after) resolves to `Endpoint::Cluster` — do a pre-pass collecting subgraph keys. `classDef name k:v,k:v` supports `fill`, `stroke`, `color`, `stroke-width` (px), `stroke-dasharray` (any value → `dashed`); colours accept `#rgb`, `#rrggbb`, `#rrggbbaa` and the CSS names `white black red green blue gray grey yellow orange purple pink`; unknown keys ignored silently, unparseable colour values produce a diagnostic. `class A,B name`, `A:::name`, `style A fill:…`, `linkStyle 0,1 stroke:…` and `linkStyle default …` apply. `click …`, `accTitle`, `accDescr` lines are skipped silently. Frontmatter `config.flowchart.*` and `%%{init}%%` are skipped (authored themes are not honoured; our style file rules).

Recovery: on an unexpected token, emit `Diagnostic { line (1-based), column (1-based, in chars), span of the offending token, message }`, skip to the next `Newline`/`Semi`, continue. Unclosed shape bracket: message `"Unclosed node label"`, node kept with the text so far. Unclosed `subgraph` at EOF: message `"Subgraph is missing end"`, cluster kept. Stray `end`: message `"Unexpected end"`.

- [ ] **Step 1: Write failing tests** (in `parser.rs`)

```rust
#[test] fn subgraph_membership_title_direction() { let gr = g("flowchart TB\nsubgraph ws[Your workspace]\n direction LR\n A --> B\n subgraph inner\n  C\n end\nend\nB --> D");
    assert_eq!(gr.clusters.len(), 2); assert_eq!(gr.clusters[0].title, "Your workspace"); assert_eq!(gr.clusters[0].direction, Some(Direction::LR));
    assert_eq!(gr.clusters[1].title, "inner"); assert_eq!(gr.clusters[1].parent, Some(ClusterId(0)));
    assert_eq!(node(&gr, "A").cluster, Some(ClusterId(0))); assert_eq!(node(&gr, "C").cluster, Some(ClusterId(1))); assert_eq!(node(&gr, "D").cluster, None); }
#[test] fn edge_to_subgraph_id_targets_cluster() { let gr = g("flowchart TB\nA --> ws\nsubgraph ws\n B\nend");
    assert_eq!(gr.edges[0].to, Endpoint::Cluster(ClusterId(0))); assert!(gr.nodes.iter().all(|n| n.key != "ws")); }
#[test] fn class_and_style() { let gr = g("flowchart TB\nclassDef hot fill:#fee,stroke:#c00,stroke-width:2px\nA:::hot --> B\nclass B hot\nstyle B color:#123456\nlinkStyle 0 stroke:#00f");
    assert_eq!(gr.class_defs[0].props.fill, Some([0xff,0xee,0xee,0xff])); assert_eq!(gr.class_defs[0].props.stroke_width, Some(2.0));
    assert_eq!(node(&gr, "A").classes, vec![0]); assert_eq!(node(&gr, "B").classes, vec![0]);
    assert_eq!(node(&gr, "B").style.color, Some([0x12,0x34,0x56,0xff])); assert_eq!(gr.edges[0].style.stroke, Some([0,0,0xff,0xff])); }
#[test] fn recovers_and_reports_line() { let p = parse("flowchart TB\nA --> B\nB --> --> C\nC --> D");
    assert_eq!(p.diagnostics.len(), 1); assert_eq!(p.diagnostics[0].line, 3); assert_eq!(p.graph.edges.len(), 2); }
#[test] fn unclosed_things_are_diagnosed_not_fatal() { let p = parse("flowchart TB\nsubgraph s\nA[open\nB");
    let m: Vec<_> = p.diagnostics.iter().map(|d| d.message.as_str()).collect(); assert!(m.contains(&"Unclosed node label")); assert!(m.contains(&"Subgraph is missing end"));
    assert_eq!(p.graph.clusters.len(), 1); }
#[test] fn crlf_and_unicode_spans() { let s = "flowchart LR\r\nA[Заказ 🚀] --> B\r\nB --> --> C\r\n"; let p = parse(s);
    let a = &p.graph.nodes[0]; assert_eq!(&s[a.label_span.start as usize..a.label_span.end as usize], "Заказ 🚀"); assert_eq!(p.diagnostics[0].line, 3);
    for n in &p.graph.nodes { assert!(s.is_char_boundary(n.span.start as usize) && s.is_char_boundary(n.span.end as usize)); } }
#[test] fn empty_inputs() { for s in ["", "flowchart LR", "flowchart LR\n%% only a comment\n", "\n\n"] { let p = parse(s); assert!(p.graph.nodes.is_empty() && p.diagnostics.is_empty(), "{s:?}"); } }
#[test] fn ignored_lines() { let gr = g("---\nconfig:\n  theme: dark\n---\n%%{init: {'theme':'forest'}}%%\nflowchart TB\nA --> B\nclick A href \"https://x\"\naccTitle: hi"); assert_eq!(gr.nodes.len(), 2); }
```

`tests/never_panic.rs`: take every `tests/fixtures/*.mmd` plus the strings above; for 2,000 iterations per seed with a fixed xorshift seed, mutate (delete a byte range, duplicate a range, insert one of `[ ] ( ) { } | - > & " \n ; : % é`), run `parse` on `String::from_utf8_lossy`, and assert only that it returns and that every span satisfies `start <= end <= len` on char boundaries.

- [ ] **Step 2:** Run. Expected: failures.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p mermaidlite-diagram`. Expected: all pass, `never_panic` finishes in under 10 s.
- [ ] **Step 5:** Commit.

---

### Task 5: Text measurement and wrapping

**Files:** Create `crates/diagram/src/text.rs`, `crates/diagram/src/metrics.rs`; add `ttf-parser = "0.25"` (use the version already in `Cargo.lock` if one is present: `grep -A1 'name = "ttf-parser"' Cargo.lock`) and `serde.workspace = true`.

**Interfaces — Produces:**

```rust
// metrics.rs
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)] #[serde(default)]
pub struct Metrics { pub font_size: f32, pub line_height: f32, pub max_label_width: f32, pub pad_x: f32, pub pad_y: f32,
    pub radius: f32, pub rank_gap: f32, pub node_gap: f32, pub edge_label_font: f32, pub cluster_pad: f32, pub cluster_title: f32,
    pub width_step: f32 }   // Default: 14, 20, 220, 18, 12, 12, 64, 40, 13, 24, 28, 24
// text.rs
pub struct Measurer { /* face bytes: Arc<[u8]>, units_per_em, ascii advances [u16;128], cache: Mutex<HashMap<char,u16>> */ }
pub struct TextBlock { pub lines: Vec<String>, pub width: f32, pub height: f32 }
impl Measurer {
    pub fn new(font: std::sync::Arc<[u8]>) -> Result<Self, String>;
    pub fn width(&self, text: &str, size: f32) -> f32;   // sum of advances, no kerning; missing glyph → advance of '?'
    pub fn wrap(&self, text: &str, size: f32, line_height: f32, max_width: f32) -> TextBlock;
}
```

`wrap`: split on `'\n'` first; within a paragraph break at spaces; a single word wider than `max_width` stays on its own line unbroken (never split words). After greedy wrapping into `k` lines, re-wrap with the smallest width `w ≤ max_width` (binary search, 8 iterations) that still gives `k` lines — this balances line lengths. `width` = widest line, `height` = lines × `line_height`. Empty text → one empty line, width 0.

- [ ] **Step 1: Write failing tests**

```rust
fn m() -> Measurer { let p = ["../../assets/fonts/Inter-Regular.ttf", "../../assets/fonts/Manrope.ttf"].into_iter().map(|p| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(p)).find(|p| p.exists()).unwrap();
    Measurer::new(std::fs::read(p).unwrap().into()).unwrap() }
#[test] fn width_scales_linearly_and_adds() { let m = m(); let a = m.width("Hello", 14.0); assert!(a > 20.0 && a < 50.0);
    assert!((m.width("Hello", 28.0) - 2.0 * a).abs() < 0.01); assert!((m.width("HelloHello", 14.0) - 2.0 * a).abs() < 0.01); }
#[test] fn short_text_one_line() { let b = m().wrap("Project index", 14.0, 20.0, 220.0); assert_eq!(b.lines, ["Project index"]); assert_eq!(b.height, 20.0); }
#[test] fn long_text_wraps_balanced_under_max() { let m = m(); let b = m.wrap("Validate the incoming request payload against the published schema", 14.0, 20.0, 220.0);
    assert!(b.lines.len() >= 2); assert!(b.width <= 220.0);
    let w: Vec<f32> = b.lines.iter().map(|l| m.width(l, 14.0)).collect(); let (mx, mn) = (w.iter().cloned().fold(0.0, f32::max), w.iter().cloned().fold(f32::MAX, f32::min));
    assert!(mn >= mx * 0.5, "unbalanced: {w:?}"); assert_eq!(b.lines.join(" "), "Validate the incoming request payload against the published schema"); }
#[test] fn explicit_newlines_and_long_word() { let b = m().wrap("a\nb", 14.0, 20.0, 220.0); assert_eq!(b.lines, ["a", "b"]);
    let b = m().wrap("Supercalifragilisticexpialidocious_and_then_some_more_text", 14.0, 20.0, 120.0); assert_eq!(b.lines.len(), 1); assert!(b.width > 120.0); }
#[test] fn non_ascii_and_empty() { let m = m(); assert!(m.width("Заказ 🚀", 14.0) > 0.0); let b = m.wrap("", 14.0, 20.0, 220.0); assert_eq!((b.lines.len(), b.width), (1, 0.0)); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 6: Shape geometry

**Files:** Create `crates/diagram/src/shape.rs`.

**Interfaces — Produces:**

```rust
#[derive(Clone, Copy, PartialEq, Debug, Default)] pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
impl Rect { pub fn center(self) -> [f32;2]; pub fn right(self) -> f32; pub fn bottom(self) -> f32; pub fn intersects(self, o: Rect) -> bool; pub fn contains(self, p: [f32;2]) -> bool; pub fn union(self, o: Rect) -> Rect; pub fn inflate(self, d: f32) -> Rect; }
pub enum PathCmd { Move([f32;2]), Line([f32;2]), Cubic([f32;2],[f32;2],[f32;2]), Close }
pub fn size(kind: ShapeKind, text: &TextBlock, m: &Metrics) -> [f32;2];
pub fn outline(kind: ShapeKind, r: Rect, radius: f32) -> Vec<PathCmd>;     // closed path in world units
pub fn boundary(kind: ShapeKind, r: Rect, toward: [f32;2]) -> [f32;2];     // where the ray centre→toward leaves the shape
```

Size rules (w, h from text + padding; `t` = text block): Rect/Rounded/Subroutine: `(t.w + 2·pad_x, t.h + 2·pad_y)`, Subroutine adds 16 to w. Stadium: w adds `h/2`. Circle/DoubleCircle: diameter `max(t.w, t.h) + 2·pad_x` (+10 for double). Diamond: `(t.w + t.h + 2·pad_x, t.w·0.5 + t.h + 2·pad_y)` clamped so h ≥ 56. Hexagon, Parallelogram(+Alt), Trapezoid(+Alt), Asymmetric: w adds `h·0.5`. Cylinder: h adds 16. Minimum for every shape: 72 × 40. Outline: Rect and Rounded both use `radius` (spec: no sharp corners; Rounded uses `radius·1.5` capped at h/2); Diamond and Hexagon and the slanted shapes round each vertex with a 4-unit cubic fillet. `boundary`: exact for rect-like (rounded-rect intersection may ignore the corner arc), ellipse formula for Circle/Stadium ends, polygon ray-cast for Diamond/Hexagon/slanted.

- [ ] **Step 1: Write failing tests**

```rust
fn tb(w: f32, lines: usize) -> TextBlock { TextBlock { lines: vec![String::new(); lines], width: w, height: lines as f32 * 20.0 } }
#[test] fn sizes() { let m = Metrics::default();
    assert_eq!(size(ShapeKind::Rect, &tb(100.0, 1), &m), [136.0, 44.0]); assert_eq!(size(ShapeKind::Rect, &tb(10.0, 1), &m), [72.0, 44.0]);
    let c = size(ShapeKind::Circle, &tb(60.0, 1), &m); assert_eq!(c[0], c[1]);
    let d = size(ShapeKind::Diamond, &tb(100.0, 1), &m); assert!(d[0] > 136.0 && d[1] >= 56.0); }
#[test] fn boundary_on_edge_for_every_shape() { use ShapeKind::*; let r = Rect { x: 0.0, y: 0.0, w: 160.0, h: 80.0 };
    for k in [Rect, Rounded, Stadium, Subroutine, Cylinder, Circle, DoubleCircle, Diamond, Hexagon, Parallelogram, ParallelogramAlt, Trapezoid, TrapezoidAlt, Asymmetric] {
        for t in [[80.0, -500.0], [80.0, 500.0], [-500.0, 40.0], [500.0, 40.0], [400.0, 300.0]] {
            let p = boundary(k, r, t); assert!(r.inflate(0.5).contains(p), "{k:?} {t:?} {p:?}");
            let c = r.center(); let d = ((p[0]-c[0]).powi(2) + (p[1]-c[1]).powi(2)).sqrt(); assert!(d >= 20.0, "{k:?} boundary collapsed to centre"); } } }
#[test] fn diamond_boundary_is_on_its_edge_not_the_bbox_corner() { let r = Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }; let p = boundary(ShapeKind::Diamond, r, [200.0, 200.0]); assert!((p[0] - 75.0).abs() < 1.0 && (p[1] - 75.0).abs() < 1.0); }
#[test] fn outlines_are_closed_and_inside_rect() { let r = Rect { x: 10.0, y: 10.0, w: 160.0, h: 80.0 };
    for k in [ShapeKind::Rect, ShapeKind::Diamond, ShapeKind::Hexagon, ShapeKind::Stadium, ShapeKind::Cylinder] { let o = outline(k, r, 12.0);
        assert!(matches!(o.first(), Some(PathCmd::Move(_))) && matches!(o.last(), Some(PathCmd::Close)));
        for c in &o { let pts: Vec<[f32;2]> = match c { PathCmd::Move(p) | PathCmd::Line(p) => vec![*p], PathCmd::Cubic(a,b,c) => vec![*a,*b,*c], PathCmd::Close => vec![] }; for p in pts { assert!(r.inflate(0.5).contains(p), "{k:?}"); } } } }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 7: Layout driver, cycle removal, ranking

**Files:** Create `crates/diagram/src/layout/{mod,acyclic,rank}.rs`.

**Interfaces — Produces:**

```rust
// layout/mod.rs — public
pub enum LayoutError { Canceled, Budget(String) }
pub struct EdgePath { pub points: Vec<[f32;2]>,           // cubic Bézier chain: 1 + 3k points, start→end in authored direction
                      pub label: Option<Rect> }
pub struct Layout { pub nodes: Vec<Rect>, pub node_text: Vec<TextBlock>, pub clusters: Vec<Rect>, pub edges: Vec<EdgePath>,
                    pub edge_text: Vec<Option<TextBlock>>, pub bounds: Rect,
                    pub slots: Vec<(u32, u32)> }           // per node: (rank, position in rank) — the stability seed
pub fn layout(g: &Graph, measurer: &Measurer, m: &Metrics, seed: Option<(&Graph, &Layout)>, cancel: &AtomicBool) -> Result<Layout, LayoutError>;

// layout/mod.rs — crate-internal working graph, direction-normalised to top→bottom
pub(crate) struct LNode { pub kind: LKind, pub w: f32, pub h: f32, pub rank: i32, pub order: u32, pub x: f32, pub y: f32, pub cluster: Option<ClusterId> }
pub(crate) enum LKind { Real(NodeId), Dummy(EdgeId), Label(EdgeId), BorderTop(ClusterId), BorderBottom(ClusterId), BorderLeft(ClusterId), BorderRight(ClusterId) }
pub(crate) struct LEdge { pub from: u32, pub to: u32, pub orig: EdgeId, pub reversed: bool, pub weight: u32, pub min_len: u32 }
pub(crate) struct LGraph { pub nodes: Vec<LNode>, pub edges: Vec<LEdge>, pub ranks: Vec<Vec<u32>> }
// acyclic.rs
pub(crate) fn make_acyclic(lg: &mut LGraph);              // flips `from`/`to` and sets `reversed` on a greedy feedback arc set
// rank.rs
pub(crate) fn assign_ranks(lg: &mut LGraph, cancel: &AtomicBool) -> Result<(), LayoutError>;
pub(crate) fn insert_dummies(lg: &mut LGraph, g: &Graph); // long edges → chains of Dummy; an edge with a label gets one Label node at its middle rank
```

Decisions: LR/RL are laid out as TB with node w/h swapped, then transposed at the end; BT/RL mirror the rank axis at the end. Self-loops are removed from `LGraph` before `make_acyclic` and handled only in routing (Task 10). `make_acyclic` uses Eades–Lin–Smyth greedy ordering (sinks right, sources left, else max out−in); ties broken by node index so output is deterministic. `assign_ranks`: longest-path initial ranking, then network simplex with the tight-tree/cut-value method when `nodes ≤ 2,000`, capped at `4·nodes` pivots; above 2,000 nodes keep longest-path and run one pass that pulls each node with all-slack in-edges toward its successors. Edge `min_len` from `Edge::min_len`; a labelled edge has `min_len ≥ 2` so the label gets its own rank. Ranks normalised to start at 0.

In this task `layout()` runs acyclic → rank → dummies and then places nodes naively (rank·(max h + rank_gap), order·(max w + node_gap)) so later tasks have a running pipeline to replace piece by piece. `edges` get a straight 4-point path centre to centre.

- [ ] **Step 1: Write failing tests** (in `layout/mod.rs`, helpers shared by later tasks)

```rust
pub(crate) fn test_measurer() -> Measurer { /* same font lookup as the `m()` helper in text.rs tests; move that helper here and reuse it there */ }
pub(crate) fn lay(src: &str) -> (Graph, Layout) { let g = crate::parse(src).graph; let l = layout(&g, &test_measurer(), &Metrics::default(), None, &AtomicBool::new(false)).ok().unwrap(); (g, l) }
fn rank_of(g: &Graph, l: &Layout, key: &str) -> u32 { l.slots[g.nodes.iter().position(|n| n.key == key).unwrap()].0 }
#[test] fn chain_ranks_increase() { let (g, l) = lay("flowchart TB\nA-->B-->C"); assert!(rank_of(&g,&l,"A") < rank_of(&g,&l,"B") && rank_of(&g,&l,"B") < rank_of(&g,&l,"C")); }
#[test] fn cycle_reverses_exactly_one_edge() { let (g, l) = lay("flowchart TB\nA-->B-->C-->A"); let r = |k| rank_of(&g,&l,k);
    let back = [("A","B"),("B","C"),("C","A")].iter().filter(|(a,b)| r(a) > r(b)).count(); assert_eq!(back, 1); }
#[test] fn network_simplex_is_tighter_than_longest_path() { // D has one parent A and one child E; longest-path leaves it at rank 1, simplex is free to; total edge length must be minimal
    let (g, l) = lay("flowchart TB\nA-->B-->C-->E\nA-->D-->E"); let r = |k| rank_of(&g,&l,k) as i32;
    let total: i32 = [("A","B"),("B","C"),("C","E"),("A","D"),("D","E")].iter().map(|(a,b)| r(b) - r(a)).sum(); assert_eq!(total, 6); }
#[test] fn extra_length_and_labels_take_ranks() { let (g, l) = lay("flowchart TB\nA--->B\nA-->|yes|C\nA-->D"); assert_eq!(rank_of(&g,&l,"B") - rank_of(&g,&l,"A"), 2); assert_eq!(rank_of(&g,&l,"C") - rank_of(&g,&l,"A"), 2); assert_eq!(rank_of(&g,&l,"D") - rank_of(&g,&l,"A"), 1); }
#[test] fn self_loop_and_two_cycle_do_not_break() { let (_, l) = lay("flowchart TB\nA-->A\nA-->B\nB-->A"); assert_eq!(l.nodes.len(), 2); assert_eq!(l.edges.len(), 3); }
#[test] fn cancel_and_budget() { let g = crate::parse("flowchart TB\nA-->B").graph; let c = AtomicBool::new(true);
    assert!(matches!(layout(&g, &test_measurer(), &Metrics::default(), None, &c), Err(LayoutError::Canceled)));
    let mut big = Graph::default(); for i in 0..20_001 { big.nodes.push(Node { key: i.to_string(), label: i.to_string(), shape: ShapeKind::Rect, cluster: None, classes: vec![], style: Default::default(), span: Span::default(), label_span: Span::default() }); }
    assert!(matches!(layout(&big, &test_measurer(), &Metrics::default(), None, &AtomicBool::new(false)), Err(LayoutError::Budget(_)))); }
#[test] fn deterministic() { let s = "flowchart TB\nA-->B & C & D-->E\nE-->A"; let (_, a) = lay(s); let (_, b) = lay(s); assert_eq!(a.nodes, b.nodes); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 8: Crossing minimisation

**Files:** Create `crates/diagram/src/layout/order.rs`; call it from `layout()` after `insert_dummies`.

**Interfaces — Produces:**

```rust
pub(crate) fn order(lg: &mut LGraph, seed: Option<&[(u32, u32)]>, cancel: &AtomicBool) -> Result<(), LayoutError>; // fills lg.ranks and LNode::order
pub(crate) fn crossings(lg: &LGraph) -> u64;   // bilayer accumulator-tree count summed over rank pairs, O(E log V)
```

Algorithm: initial order by DFS from rank-0 nodes in node-index order (or from `seed` — Task 13). Then sweeps alternating down/up using the weighted median of neighbour positions (barycentre for ties, nodes with no neighbours keep their slot), each followed by adjacent-transpose passes that swap while crossings drop. Keep the best ordering seen. Sweep budget: 24 when `nodes ≤ 500`, 8 when `≤ 5,000`, else 4; stop early after 4 sweeps without improvement.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn counts_crossings() { /* build LGraph by hand: ranks [[a,b],[c,d]], edges a→d, b→c */ assert_eq!(crossings(&x_graph()), 1); assert_eq!(crossings(&parallel_graph()), 0); }
#[test] fn untangles_a_simple_x() { let (g, l) = lay("flowchart TB\nA-->D\nB-->C\nA-->C2\nB-->D2"); /* A,B on rank 0 */
    let x = |k: &str| l.nodes[g.nodes.iter().position(|n| n.key == k).unwrap()].x; assert_eq!((x("A") < x("B")), (x("D") < x("C"))); }
#[test] fn tree_has_zero_crossings() { let (g, l) = lay("flowchart TB\nR-->A & B & C\nA-->A1 & A2\nB-->B1 & B2\nC-->C1 & C2"); assert_eq!(edge_crossings(&g, &l), 0); }
#[test] fn ladder_has_zero_crossings() { let (g, l) = lay("flowchart TB\nA1-->A2-->A3-->A4\nB1-->B2-->B3-->B4\nA1-->B2\nA2-->B3\nA3-->B4"); assert_eq!(edge_crossings(&g, &l), 0); }
```

`edge_crossings(&Graph, &Layout) -> usize` is a test helper counting pairwise intersections of straight centre-to-centre segments between edges that share no endpoint; put it next to `lay` for reuse in Task 15.

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 9: Coordinate assignment (Brandes–Köpf)

**Files:** Create `crates/diagram/src/layout/position.rs`; replace the naive placement in `layout()`.

**Interfaces — Produces:** `pub(crate) fn position(lg: &mut LGraph, m: &Metrics)` — fills `LNode::x` (centre) and `LNode::y` (centre).

Decisions: y per rank = running sum of (tallest node in rank + `rank_gap`); a rank containing only dummies/labels uses the label height or 0. x by Brandes–Köpf: mark type-1 conflicts (non-inner segment crossing an inner dummy–dummy segment), run vertical alignment + horizontal compaction for the four direction combinations with separation `(w_a + w_b)/2 + node_gap` (dummy–dummy separation `node_gap/2`), align the four results to the narrowest, take the average of the two median x per node. Finally translate so the minimum left edge is 0.

- [ ] **Step 1: Write failing tests**

```rust
fn cx(g: &Graph, l: &Layout, k: &str) -> f32 { let r = l.nodes[g.nodes.iter().position(|n| n.key == k).unwrap()]; r.x + r.w / 2.0 }
#[test] fn no_overlap_and_min_gap_within_rank() { let (g, l) = lay("flowchart TB\nR-->A[Short] & B[A much longer label here] & C & D"); let m = Metrics::default();
    let mut row: Vec<Rect> = ["A","B","C","D"].iter().map(|k| l.nodes[g.nodes.iter().position(|n| n.key == *k).unwrap()]).collect(); row.sort_by(|a, b| a.x.total_cmp(&b.x));
    for w in row.windows(2) { assert!(w[1].x - w[0].right() >= m.node_gap - 0.01); } }
#[test] fn parent_centred_over_children() { let (g, l) = lay("flowchart TB\nR-->A & B & C"); let mid = (cx(&g,&l,"A") + cx(&g,&l,"C")) / 2.0; assert!((cx(&g,&l,"R") - mid).abs() < 1.0); assert!((cx(&g,&l,"B") - mid).abs() < 1.0); }
#[test] fn chain_is_perfectly_straight() { let (g, l) = lay("flowchart TB\nA-->B[Wider label]-->C-->D"); let x = cx(&g,&l,"A"); for k in ["B","C","D"] { assert!((cx(&g,&l,k) - x).abs() < 0.5); } }
#[test] fn long_edge_dummies_are_vertical() { let (g, l) = lay("flowchart TB\nA-->B-->C-->D\nA-->D"); let e = &l.edges[3]; /* after Task 10 this checks spline; for now: */ let _ = (g, e); }
#[test] fn rank_gap_respected() { let (g, l) = lay("flowchart TB\nA-->B"); let a = l.nodes[0]; let b = l.nodes[1]; let _ = g; assert!((b.y - a.bottom() - Metrics::default().rank_gap).abs() < 0.01); }
#[test] fn lr_swaps_axes() { let (_, l) = lay("flowchart LR\nA-->B"); assert!(l.nodes[1].x > l.nodes[0].right()); assert!((l.nodes[0].y - l.nodes[1].y).abs() < 1.0); }
#[test] fn bt_and_rl_mirror() { let (_, l) = lay("flowchart BT\nA-->B"); assert!(l.nodes[1].bottom() < l.nodes[0].y); let (_, l) = lay("flowchart RL\nA-->B"); assert!(l.nodes[1].right() < l.nodes[0].x); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 10: Edge routing

**Files:** Create `crates/diagram/src/layout/route.rs`; replace straight edges in `layout()`; fill `edge_text` and `EdgePath::label`.

**Interfaces — Produces:** `pub(crate) fn route(lg: &LGraph, g: &Graph, nodes: &[Rect], m: &Metrics) -> Vec<EdgePath>`.

Decisions:
- Waypoints: source centre, each dummy/label centre in rank order, target centre. First and last are replaced by `shape::boundary(kind, rect, next_waypoint)`.
- Ports: edges leaving the same side of a node are sorted by the x of their next waypoint and their start points spread across the middle 60% of that side (same for arrivals), then re-clipped to the outline.
- Curve: waypoints → cubic chain with vertical tangents at ends (control points offset along the rank axis by 40% of the rank distance), Catmull-Rom-derived tangents at interior waypoints clamped so the curve stays monotone along the rank axis.
- Reversed edges (`LEdge::reversed`) are routed in layout direction then the point list is reversed, so `points[0]` is always at the authored source.
- Edge label rect = the Label node's rect (text block + 6 px horizontal, 3 px vertical padding), centred on its waypoint.
- Self-loop: a loop on the node's right side, cubic chain leaving at 30% height and returning at 70%, bulging `node_gap·0.8` outward; its label (if any) sits right of the bulge. Extra `node_gap` is reserved to the right of a node with a self-loop (add to its width in `LGraph` before `position`, subtract when writing `Layout::nodes`).
- Parallel edges between the same pair are separate dummy chains already when long; when adjacent-rank, offset their interior control points by ±8 px per duplicate.

- [ ] **Step 1: Write failing tests**

```rust
fn on_outline(g: &Graph, l: &Layout, key: &str, p: [f32;2]) -> bool { let i = g.nodes.iter().position(|n| n.key == key).unwrap(); let r = l.nodes[i];
    let b = crate::shape::boundary(g.nodes[i].shape, r, p); r.inflate(1.0).contains(p) && !r.inflate(-6.0).contains(p) || ((b[0]-p[0]).abs() < 2.0 && (b[1]-p[1]).abs() < 2.0) }
#[test] fn endpoints_touch_shapes_in_authored_direction() { let (g, l) = lay("flowchart TB\nA-->B{Ok?}\nB-->C((Done))\nC-->A"); for (i, (s, t)) in [("A","B"),("B","C"),("C","A")].iter().enumerate() {
    let p = &l.edges[i].points; assert_eq!((p.len() - 1) % 3, 0); assert!(on_outline(&g,&l,s,p[0]), "start {i}"); assert!(on_outline(&g,&l,t,*p.last().unwrap()), "end {i}"); } }
#[test] fn straight_chain_edge_is_a_vertical_line() { let (_, l) = lay("flowchart TB\nA-->B"); let p = &l.edges[0].points; assert!(p.iter().all(|q| (q[0] - p[0][0]).abs() < 0.5)); }
#[test] fn curve_is_monotone_along_rank_axis() { let (_, l) = lay("flowchart TB\nA-->B-->C-->D\nA-->D\nA-->C"); for e in &l.edges { for w in e.points.windows(2) { assert!(w[1][1] >= w[0][1] - 0.01); } } }
#[test] fn label_rect_clear_of_nodes_and_other_labels() { let (_, l) = lay("flowchart TB\nA-->|first choice|B\nA-->|second choice|C\nA-->|third|D"); let labels: Vec<Rect> = l.edges.iter().map(|e| e.label.unwrap()).collect();
    for (i, a) in labels.iter().enumerate() { for n in &l.nodes { assert!(!a.intersects(*n)); } for b in &labels[i+1..] { assert!(!a.intersects(*b)); } } }
#[test] fn ports_spread() { let (_, l) = lay("flowchart TB\nA[A fairly wide parent node]-->B & C & D"); let xs: Vec<f32> = l.edges.iter().map(|e| e.points[0][0]).collect(); assert!(xs[0] != xs[1] && xs[1] != xs[2]); }
#[test] fn self_loop_leaves_and_returns_on_same_node() { let (g, l) = lay("flowchart TB\nA-->A\nA-->B"); let p = &l.edges[0].points; assert!(on_outline(&g,&l,"A",p[0]) && on_outline(&g,&l,"A",*p.last().unwrap())); assert!(p.iter().any(|q| q[0] > l.nodes[0].right() + 8.0)); }
#[test] fn parallel_edges_differ() { let (_, l) = lay("flowchart TB\nA-->B\nA-->B"); assert_ne!(l.edges[0].points, l.edges[1].points); }
#[test] fn two_cycle_keeps_authored_ends() { let (g, l) = lay("flowchart TB\nA-->B\nB-->A"); assert!(on_outline(&g,&l,"B",l.edges[1].points[0])); assert!(on_outline(&g,&l,"A",*l.edges[1].points.last().unwrap())); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 11: Subgraphs (compound layout)

**Files:** Create `crates/diagram/src/layout/compound.rs`; hooks in `rank.rs`, `order.rs`, `position.rs`, `route.rs`, `mod.rs`.

**Interfaces — Produces:**

```rust
pub(crate) fn add_borders(lg: &mut LGraph, g: &Graph);          // before ranking: BorderTop/BorderBottom per cluster, nesting edges
pub(crate) fn add_side_borders(lg: &mut LGraph, g: &Graph);     // after ranking: BorderLeft/BorderRight per cluster per rank it spans
pub(crate) fn cluster_rects(lg: &LGraph, g: &Graph, m: &Metrics) -> Vec<Rect>;
pub(crate) fn layout_nested(g: &Graph, c: ClusterId, …) -> (Graph /*sub*/, Layout); // recursive path for clusters with their own direction
```

Decisions:
- **Global path** (cluster has no `direction`, or it equals the parent's): `BorderTop → every member` and `every member → BorderBottom` edges with weight 0, min_len 1; nested clusters' borders are members of the parent. Top border rank gets `cluster_title + cluster_pad` extra height; bottom gets `cluster_pad`. After ranking, one BorderLeft and one BorderRight per rank between top and bottom.
- **Ordering constraint**: within each rank, members of a cluster (including its side borders) must be contiguous with BorderLeft first and BorderRight last, recursively. Implement by sorting cluster subtrees as units by their barycentre in each sweep; transpose never swaps across a cluster boundary.
- **Position**: side borders are width-0 nodes with separation `cluster_pad` to members; after `position`, `cluster_rects` = bbox of members and borders, inflated so the title (measured at 13 px semibold ≈ `width(title, 13)·1.06`) fits; sibling clusters never overlap because borders occupy slots.
- **Recursive path** (cluster declares a different `direction`): build a sub-`Graph` of its members and internal edges, call `layout` recursively, insert one `LNode` of that size in the parent, then translate the sub-layout into place. Edges crossing into it attach to the cluster rect boundary nearest the other end, then continue as a straight-then-curved segment to the inner node's boundary.
- **Edge to a cluster** (`Endpoint::Cluster`): in the global path it attaches to BorderTop (incoming) or BorderBottom (outgoing) and is clipped to the cluster rect; in the recursive path, to the stand-in node.
- Empty subgraph: a 160 × 72 container with just its title.
- `Layout::clusters[i]` is the rect for `ClusterId(i)`. `bounds` includes clusters.

- [ ] **Step 1: Write failing tests**

```rust
fn rect(g: &Graph, l: &Layout, k: &str) -> Rect { l.nodes[g.nodes.iter().position(|n| n.key == k).unwrap()] }
fn inside(o: Rect, i: Rect) -> bool { i.x >= o.x && i.y >= o.y && i.right() <= o.right() && i.bottom() <= o.bottom() }
#[test] fn members_inside_with_padding_and_title_room() { let (g, l) = lay("flowchart TB\nsubgraph ws[Your local workspace]\nA-->B\nend\nB-->C"); let c = l.clusters[0]; let m = Metrics::default();
    for k in ["A","B"] { assert!(inside(c.inflate(-m.cluster_pad + 0.5), rect(&g,&l,k))); } assert!(rect(&g,&l,"A").y - c.y >= m.cluster_title + m.cluster_pad - 0.5); assert!(!c.intersects(rect(&g,&l,"C"))); }
#[test] fn outsiders_never_inside_a_cluster() { let (g, l) = lay("flowchart TB\nX-->A\nX-->Y\nsubgraph s\nA-->B\nend\nY-->Z\nB-->Z"); for k in ["X","Y","Z"] { assert!(!l.clusters[0].intersects(rect(&g,&l,k)), "{k}"); } }
#[test] fn siblings_do_not_overlap_and_nested_is_contained() { let (_, l) = lay("flowchart TB\nsubgraph a\nA1-->A2\nsubgraph inner\nI1-->I2\nend\nA2-->I1\nend\nsubgraph b\nB1-->B2\nend\nA1-->B1");
    assert!(!l.clusters[0].intersects(l.clusters[2])); assert!(inside(l.clusters[0], l.clusters[1])); }
#[test] fn own_direction_lays_out_sideways() { let (g, l) = lay("flowchart TB\nsubgraph s\ndirection LR\nA-->B-->C\nend\nS-->A"); assert!(rect(&g,&l,"B").x > rect(&g,&l,"A").right()); assert!((rect(&g,&l,"A").y - rect(&g,&l,"C").y).abs() < 1.0); assert!(rect(&g,&l,"S").bottom() < l.clusters[0].y); }
#[test] fn edge_to_cluster_ends_on_cluster_border() { let (_, l) = lay("flowchart TB\nA-->ws\nsubgraph ws\nB-->C\nend"); let end = *l.edges[0].points.last().unwrap(); let c = l.clusters[0]; assert!((end[1] - c.y).abs() < 1.5 && end[0] >= c.x && end[0] <= c.right()); }
#[test] fn empty_subgraph_has_a_box() { let (_, l) = lay("flowchart TB\nsubgraph lonely[Nothing here yet]\nend\nA"); assert!(l.clusters[0].w >= 160.0 && l.clusters[0].h >= 72.0); }
#[test] fn example_system_file_lays_out() { let s = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/system.mmd")).unwrap(); let p = crate::parse(&s); assert!(p.diagnostics.is_empty()); let (g, l) = lay(&s);
    for n in ["Files","Index","Search","Editor"] { assert!(inside(l.clusters[0], rect(&g,&l,n))); } for n in ["Helper","Preview","Recovery","Export"] { assert!(!l.clusters[0].intersects(rect(&g,&l,n))); } }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run the whole suite → all earlier tests still pass. **Step 5:** Commit.

---

### Task 12: Component packing and width snapping

**Files:** Create `crates/diagram/src/layout/pack.rs`; `layout()` splits into weakly connected components first (a cluster and everything in it count as connected), lays each out, then packs.

**Interfaces — Produces:**

```rust
pub(crate) fn snap_width(w: f32, step: f32) -> f32;                          // round up to a multiple of `step`
pub(crate) fn pack(sizes: &[[f32;2]], gap: f32, target_aspect: f32) -> Vec<[f32;2]>; // top-left offset per component
```

Decisions: `snap_width` is applied to every real node's width before ranking (`Metrics::width_step` = 24), so siblings with similar labels get equal widths. `pack`: sort components by height descending (stable), shelf-pack into rows, choosing the row width that brings total bbox aspect closest to `target_aspect` = 1.6 (try each prefix-sum breakpoint, at most 64 candidates); gap between components = `rank_gap`. A single component returns `[0,0]`. For LR/RL the packing still targets 1.6 on the final, transposed axes.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn widths_snap_up() { assert_eq!(snap_width(73.0, 24.0), 96.0); assert_eq!(snap_width(96.0, 24.0), 96.0); }
#[test] fn similar_siblings_get_equal_width() { let (_, l) = lay("flowchart TB\nR-->A[Parse input] & B[Parse inputs] & C[Parse input 2]"); assert_eq!(l.nodes[1].w, l.nodes[2].w); assert_eq!(l.nodes[2].w, l.nodes[3].w); }
#[test] fn ten_small_chains_pack_into_a_block_not_a_strip() { let s = (0..10).map(|i| format!("A{i}-->B{i}")).collect::<Vec<_>>().join("\n"); let (_, l) = lay(&format!("flowchart TB\n{s}")); let a = l.bounds.w / l.bounds.h; assert!(a > 0.8 && a < 3.2, "aspect {a}"); }
#[test] fn packed_components_do_not_overlap() { let (_, l) = lay("flowchart TB\nA-->B-->C\nD-->E\nF\nG-->H-->I-->J"); for (i, a) in l.nodes.iter().enumerate() { for b in &l.nodes[i+1..] { assert!(!a.intersects(*b)); } } }
#[test] fn bounds_cover_everything_and_start_at_origin() { let (_, l) = lay("flowchart LR\nA-->B\nC-->D\nsubgraph s\nE\nend"); assert!(l.bounds.x.abs() < 0.01 && l.bounds.y.abs() < 0.01);
    for r in l.nodes.iter().chain(&l.clusters) { assert!(inside(l.bounds, *r)); } for e in &l.edges { for p in &e.points { assert!(l.bounds.inflate(0.5).contains(*p)); } } }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 13: Stable re-layout while typing

**Files:** Modify `layout/mod.rs`, `layout/order.rs`.

**Interfaces — Consumes/Produces:** `layout(…, seed: Option<(&Graph, &Layout)>, …)`. When a seed is given: map old node keys → `slots` position; the initial order of each rank sorts nodes by `(seed position if key existed, else barycentre of already-placed neighbours, else index)`; the sweep loop accepts a new ordering only if it removes at least one crossing compared with the seeded one (strict improvement), so an equally good shuffle never replaces the seed.

- [ ] **Step 1: Write failing tests**

```rust
fn relay(src1: &str, src2: &str) -> (Graph, Layout, Graph, Layout) { let (g1, l1) = lay(src1); let g2 = crate::parse(src2).graph; let l2 = layout(&g2, &test_measurer(), &Metrics::default(), Some((&g1, &l1)), &AtomicBool::new(false)).ok().unwrap(); (g1, l1, g2, l2) }
fn left_to_right(g: &Graph, l: &Layout, keys: &[&str]) -> Vec<String> { let mut v: Vec<(f32, String)> = keys.iter().map(|k| (rect(g, l, k).x, k.to_string())).collect(); v.sort_by(|a, b| a.0.total_cmp(&b.0)); v.into_iter().map(|x| x.1).collect() }
#[test] fn adding_a_leaf_keeps_sibling_order() { let a = "flowchart TB\nR-->C & A & B\nA-->A1\nB-->B1\nC-->C1"; let b = &format!("{a}\nB-->B2");
    let (g1, l1, g2, l2) = relay(a, b); assert_eq!(left_to_right(&g1,&l1,&["A","B","C"]), left_to_right(&g2,&l2,&["A","B","C"])); }
#[test] fn renaming_a_label_moves_nothing_across() { let (g1, l1, g2, l2) = relay("flowchart TB\nR-->A[One] & B[Two] & C[Three]", "flowchart TB\nR-->A[One] & B[Two renamed to be longer] & C[Three]"); assert_eq!(left_to_right(&g1,&l1,&["A","B","C"]), left_to_right(&g2,&l2,&["A","B","C"])); }
#[test] fn seed_is_overridden_when_it_would_cross() { let (_, _, g2, l2) = relay("flowchart TB\nA-->C\nB-->D", "flowchart TB\nA-->D\nB-->C"); assert_eq!(edge_crossings(&g2, &l2), 0); }
#[test] fn seed_from_unrelated_graph_is_harmless() { let (_, _, g2, l2) = relay("flowchart LR\nX-->Y", "flowchart TB\nR-->A & B\nA-->C\nB-->C"); assert_eq!(l2.nodes.len(), g2.nodes.len()); assert_eq!(edge_crossings(&g2, &l2), 0); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 14: Scene

**Files:** Create `crates/diagram/src/scene.rs`; add `render` to `lib.rs`.

**Interfaces — Produces:**

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum ItemId { Node(NodeId), Edge(EdgeId), Cluster(ClusterId) }
pub struct NodeItem { pub id: NodeId, pub rect: Rect, pub shape: ShapeKind, pub text: TextBlock, pub span: Span, pub role_shape: ShapeKind, pub classes: Vec<u16>, pub style: StyleProps }
pub struct EdgeItem { pub id: EdgeId, pub points: Vec<[f32;2]>, pub bounds: Rect, pub line: LineKind, pub head: ArrowKind, pub tail: ArrowKind,
                      pub label: Option<(Rect, TextBlock)>, pub from: Endpoint, pub to: Endpoint, pub span: Span, pub style: StyleProps }
pub struct ClusterItem { pub id: ClusterId, pub rect: Rect, pub title: String, pub depth: u8, pub span: Span, pub classes: Vec<u16>, pub style: StyleProps }
pub struct Scene { pub bounds: Rect, pub clusters: Vec<ClusterItem>,      // paint order: clusters (outer first), edges, nodes
                   pub edges: Vec<EdgeItem>, pub nodes: Vec<NodeItem>, pub class_defs: Vec<ClassDef>, /* grid, csr adjacency: private */ }
impl Scene {
    pub fn build(g: &Graph, l: &Layout) -> Scene;
    pub fn query(&self, view: Rect, out: &mut Vec<ItemId>);               // appends, in paint order, every item whose bounds intersect `view`
    pub fn hit(&self, p: [f32;2], tolerance: f32) -> Option<ItemId>;      // nodes first, then edge labels, then edge curves within `tolerance`, then innermost cluster
    pub fn neighbours(&self, n: NodeId) -> (&[EdgeId], &[NodeId]);        // incident edges and adjacent nodes, both directions, deduplicated
    pub fn node_at_offset(&self, byte: u32) -> Option<NodeId>;            // node whose span contains the source offset (smallest span wins)
    pub fn find(&self, needle: &str) -> Vec<NodeId>;                      // case-insensitive substring over node labels, in reading order (y, then x)
    pub fn byte_size(&self) -> usize;                                     // for the cache budget in plan 2
}
// lib.rs
pub struct Rendered { pub family: Family, pub scene: Scene, pub layout: Layout, pub graph: Graph, pub diagnostics: Vec<Diagnostic> }
pub fn render(source: &str, measurer: &Measurer, m: &Metrics, seed: Option<(&Graph, &Layout)>, cancel: &AtomicBool) -> Result<Rendered, LayoutError>;
```

Decisions: uniform grid, cell size = `max(256, sqrt(area / max(items, 1)) · 2)`, each cell a `Vec<u32>` of item indices; `query` takes `&self`: collect candidates from the covered cells, sort and `dedup`, then order by paint order (clusters, edges, nodes; index within each). Edge hit test: flatten each cubic into 12 segments and take point-segment distance. Edges adjacent to a cluster endpoint contribute no node neighbour.

- [ ] **Step 1: Write failing tests**

```rust
fn sc(src: &str) -> Scene { let (g, l) = crate::layout::lay(src); Scene::build(&g, &l) }
#[test] fn query_returns_only_visible_in_paint_order() { let s = sc("flowchart TB\nsubgraph z\nA-->B\nend\nB-->C"); let mut out = vec![]; s.query(s.bounds, &mut out);
    assert_eq!(out.len(), 1 + 2 + 3); assert!(matches!(out[0], ItemId::Cluster(_))); assert!(matches!(out[1], ItemId::Edge(_))); assert!(matches!(out.last(), Some(ItemId::Node(_))));
    let a = s.nodes[0].rect; out.clear(); s.query(Rect { x: a.x + 1.0, y: a.y + 1.0, w: 2.0, h: 2.0 }, &mut out); assert!(out.contains(&ItemId::Node(NodeId(0)))); assert!(!out.contains(&ItemId::Node(NodeId(2)))); }
#[test] fn hit_priorities() { let s = sc("flowchart TB\nsubgraph z\nA-->|go|B\nend"); let a = s.nodes[0].rect; assert_eq!(s.hit(a.center(), 4.0), Some(ItemId::Node(NodeId(0))));
    let (lr, _) = s.edges[0].label.as_ref().unwrap(); assert_eq!(s.hit(lr.center(), 4.0), Some(ItemId::Edge(EdgeId(0))));
    let c = s.clusters[0].rect; assert_eq!(s.hit([c.x + 3.0, c.y + 3.0], 4.0), Some(ItemId::Cluster(ClusterId(0)))); assert_eq!(s.hit([-500.0, -500.0], 4.0), None); }
#[test] fn neighbours_both_directions() { let s = sc("flowchart TB\nA-->B\nC-->B\nB-->D\nB-->B"); let (e, n) = s.neighbours(NodeId(1)); assert_eq!(e.len(), 4); let mut n = n.to_vec(); n.sort(); assert_eq!(n, [NodeId(0), NodeId(2), NodeId(3)]); }
#[test] fn offset_to_node_and_find() { let src = "flowchart TB\nAlpha[First step] --> Beta[second STEP]"; let s = sc(src); assert_eq!(s.node_at_offset(src.find("First").unwrap() as u32), Some(NodeId(0))); assert_eq!(s.node_at_offset(2), None); assert_eq!(s.find("step"), [NodeId(0), NodeId(1)]); assert!(s.find("zzz").is_empty()); }
#[test] fn empty_scene() { let s = sc("flowchart LR"); assert_eq!((s.bounds.w, s.bounds.h), (0.0, 0.0)); let mut o = vec![]; s.query(Rect { x: -1e6, y: -1e6, w: 2e6, h: 2e6 }, &mut o); assert!(o.is_empty()); assert_eq!(s.hit([0.0, 0.0], 4.0), None); }
#[test] fn render_reports_family_and_partial_graph() { let r = crate::render("flowchart TB\nA-->B\nB --> --> C", &test_measurer(), &Metrics::default(), None, &AtomicBool::new(false)).ok().unwrap(); assert_eq!(r.family, Family::Flowchart); assert_eq!(r.diagnostics.len(), 1); assert_eq!(r.scene.nodes.len(), 2);
    let r = crate::render("sequenceDiagram\nA->>B: hi", &test_measurer(), &Metrics::default(), None, &AtomicBool::new(false)).ok().unwrap(); assert_eq!(r.family, Family::Sequence); assert!(r.scene.nodes.is_empty()); }
```

- [ ] **Step 2:** Run → fail. **Step 3:** Implement. **Step 4:** Run → pass. **Step 5:** Commit.

---

### Task 15: Fixture corpus, invariants, timing harness

**Files:**
- Create: `crates/diagram/tests/fixtures/` with at least these files — `chain.mmd`, `tree.mmd`, `diamond_decisions.mmd` (8 decision nodes with yes/no labels), `cycles.mmd`, `nested_subgraphs.mmd` (3 levels), `cross_cluster_edges.mmd`, `lr_pipeline.mmd`, `mixed_directions.mmd`, `all_shapes.mmd`, `long_labels.mmd`, `unicode.mmd`, `disconnected.mmd`, `self_loops_parallel.mmd`, `llm_architecture.mmd` (about 60 nodes, 6 subgraphs, written to resemble an LLM-generated system overview), plus a copy of `examples/system.mmd`.
- Create: `crates/diagram/tests/invariants.rs`
- Create: `tools/xtask/src/bench.rs`; modify `tools/xtask/src/main.rs` (add `bench` to the accepted actions and to the help text) and `tools/xtask/Cargo.toml` (depend on `mermaidlite-diagram`).

**Interfaces — Consumes:** `mermaidlite_diagram::{parse, render, Measurer, Metrics, Scene, Rect, Endpoint}`.

`invariants.rs` runs every fixture through `render` and asserts, with the fixture name in every message:
1. zero parse diagnostics;
2. no two node rects intersect;
3. every node with `cluster: Some(c)` is inside `clusters[c]`; every node not in cluster `c` or its descendants does not intersect `clusters[c]`;
4. sibling clusters do not intersect; child clusters are inside their parent;
5. every edge has `(points.len() − 1) % 3 == 0`, and its first/last points are within 2 units of `shape::boundary` of its endpoints (or on the cluster rect border for cluster endpoints);
6. no edge label rect intersects a node rect or another edge label rect;
7. `bounds` contains everything and starts at (0,0);
8. rendering twice gives identical node rects;
9. for TB fixtures without cycles: every edge's target rank > source rank.

It also prints one line per fixture: `name  nodes  edges  crossings  aspect` (crossings from the Task 8 helper, moved to a `pub fn` behind `#[doc(hidden)]` in `lib.rs`), so layout quality regressions are visible in test output with `--nocapture`.

`bench.rs`: `cargo native bench` builds graphs in memory as source text — `chain(n)`, `tree(n, fanout 3)`, `layered_dense(n)` (ranks of 20, each node 2 random forward edges, fixed seed), `clustered(n)` (clusters of 25 with 10% cross edges) — for n in 200, 2,000, 10,000; runs `parse`, `layout`, `Scene::build` separately, best of 5, and prints a table of milliseconds. It exits non-zero only if a run panics. Spec targets to compare against by eye: 200 nodes < 5 ms, 2,000 < 60 ms, 10,000 < 800 ms (parse + layout + scene, release build).

- [ ] **Step 1:** Write the fixtures and `invariants.rs`.
- [ ] **Step 2:** Run `cargo test -p mermaidlite-diagram --test invariants -- --nocapture`. Expected: failures are real layout bugs — fix them in the owning module (do not loosen an invariant).
- [ ] **Step 3:** Write `bench.rs` and wire the `bench` action.
- [ ] **Step 4:** Run `cargo run --release -p xtask -- bench` (check `tools/xtask/Cargo.toml` for the actual package name). Expected: a table for 4 topologies × 3 sizes with no panic. Record the table at the bottom of this plan file under "Measured".
- [ ] **Step 5:** If any 2,000-node row exceeds 60 ms or any 10,000-node row exceeds 800 ms, profile that stage and fix the hot spot (typical culprits: transpose loop, crossing count, network simplex cut-value recomputation); re-run; update the table. If a target still cannot be met, write the measured number and the reason under "Measured" rather than changing the target.
- [ ] **Step 6:** Run the full suite once more: `cargo test -p mermaidlite-diagram`. Expected: all pass.
- [ ] **Step 7:** Commit.

---

## Measured

(filled in by Task 15)
