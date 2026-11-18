# Compatibility

Last updated 18 November 2026. This describes what the in-house engine (`crates/diagram`, see [ADR 002](decisions/002-own-diagram-engine.md)) accepts. It was written from `crates/diagram/src/parser.rs` and `lexer.rs`. It is not a tested compatibility matrix: no automated tests exist in this phase, and Mermaid.js parity has not been checked.

## Diagram families

| Family | Keyword | Status |
|---|---|---|
| Flowchart | `flowchart`, `graph` | Supported |
| Sequence | `sequenceDiagram` | Not supported yet |
| State | `stateDiagram`, `stateDiagram-v2` | Not supported yet |
| Class | `classDiagram` | Not supported yet |
| Entity relationship | `erDiagram` | Not supported yet |
| Everything else (gantt, pie, journey, mindmap, timeline, git graph, and others) | any other first word | Not supported yet |

A source in an unsupported family shows a card with the title "<Family> diagrams are not supported yet" (or "This diagram type is not supported yet"). The source stays editable. There is no fallback renderer.

## Flowchart syntax

### Preamble

- A leading `---` frontmatter block is skipped.
- `%%` comment lines and `%%{ ... }%%` directives (including `%%{init: ...}%%`) are skipped, not applied.
- Blank lines and `;` separate statements.
- `accTitle` and `accDescr` (single line or `{ ... }` block) are skipped.

### Direction

`graph` or `flowchart` followed by `TB`, `TD`, `BT`, `LR` or `RL`. The default is `TB`.

### Node shapes

| Shape | Source form |
|---|---|
| Rectangle | `A[text]` or just `A` |
| Rounded | `A(text)` |
| Stadium | `A([text])` |
| Subroutine | `A[[text]]` |
| Cylinder | `A[(text)]` |
| Circle | `A((text))` |
| Double circle | `A(((text)))` |
| Asymmetric | `A>text]` |
| Diamond | `A{text}` |
| Hexagon | `A{{text}}` |
| Parallelogram | `A[/text/]` |
| Parallelogram, alternate | `A[\text\]` |
| Trapezoid | `A[/text\]` |
| Trapezoid, alternate | `A[\text/]` |

Labels may be quoted (`A["text with ) inside"]`).

The extended form `A@{ shape: name, label: "text" }` is read for `shape` and `label` only. Names: `rounded`, `event`, `stadium`, `pill`, `terminal`, `subproc`, `subroutine`, `fr-rect`, `framed-rectangle`, `cyl`, `cylinder`, `database`, `db`, `circle`, `circ`, `dbl-circ`, `double-circle`, `diam`, `diamond`, `decision`, `question`, `hex`, `hexagon`, `prepare`, `lean-r`, `lean-l`, `trap-b`, `trap-t`, `odd`. Any other name draws a rectangle. Other keys in the block are ignored.

### Edges

| Kind | Forms |
|---|---|
| Solid arrow | `-->` |
| Solid, no head | `---` |
| Dotted | `-.->`, `-.-` |
| Thick | `==>`, `===` |
| Invisible (affects layout, not drawn) | `~~~` |
| Circle or cross head | `--o`, `--x` |
| Both ends | `<-->`, `o--o`, `x--x`, `o--x` and similar |
| Longer links | extra `-`, `=` or `.` lengthen the edge by one rank each (`--->`) |

Labels: `A -->|text| B`, or inline `A -- text --> B`, `A == text ==> B`, `A -. text .-> B`. Labels may be quoted. Empty labels are dropped.

A leading `o` or `x` counts as a tail marker only when the link also ends in `o` or `x`, so a node named `x` in `x-->y` stays a node.

Chains and groups: `A --> B --> C`, `A & B --> C & D`.

### Subgraphs

- Forms: `subgraph id [Title]`, `subgraph id`, `subgraph "Title"`, `subgraph Some Title`. Close with `end`.
- Nesting to any depth.
- `direction LR` (or another direction) as the first statement of a subgraph gives that subgraph its own direction.
- Edges to and from a subgraph id (`A --> sub1`) end on the subgraph box.
- A node declared outside a subgraph and then listed inside one joins that subgraph. The first subgraph that lists it wins. Mermaid.js may place it differently.
- A subgraph key used before its declaration is resolved at the end of parsing.

### Styling

- `classDef name fill:#f9f,stroke:#333,stroke-width:2px,color:#000`. Several names may be listed with commas. A class named `default` is not special.
- `class A,B name` (nodes or subgraph ids), and the shorthand `A:::name` or `A:::one:::two` on a node.
- `style A fill:#f9f,stroke:#333` on a node or a subgraph.
- `linkStyle 0,2 stroke:#f00,stroke-width:3px`, and `linkStyle default ...`. Indices count edges in source order.
- Recognised properties: `fill`, `stroke`, `color`, `stroke-width`, `stroke-dasharray` (any value draws a dashed line).
- Colours: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `none`, `transparent`, and the names white, black, red, green, blue, gray, grey, yellow, orange, purple, pink. Other names report a diagnostic.
- Authored colours win over the app palette. When only a fill is authored, the border is derived from it and the text colour is whichever of two inks has the higher contrast.

### Labels

- `<br>`, `<br/>` and `\n` break lines.
- Other HTML tags are dropped and their text kept. HTML entities `&amp; &lt; &gt; &quot; &apos; &nbsp;` and numeric entities are decoded.
- Markdown strings (`"` + backtick form) are shown as plain text; the emphasis markers `**`, `__` and `*` are removed.
- Long labels wrap at the configured label width.

## Known gaps

- `click` statements are ignored.
- HTML labels beyond `<br>`: no links, images, lists or styling.
- Icons and images in nodes are not supported.
- Authored themes: `%%{init}%%`, frontmatter `config` and `theme` are skipped. Only `classDef`, `style` and `linkStyle` colours apply.
- Markdown strings: rendered as plain text, with no bold or italic.
- Math (`$$`), edge ids (`e1@-->`) and edge animation are not read; no test covered them.
- Diagnostics are reported per line in the info popover. A bad line is skipped and the rest still renders.
- Limits: 20,000 nodes, 20,000 edges, 2 MiB of source per diagram.
