# A little clarity

This example belongs to MermaidLite. Open the `examples` folder to explore the reading and editing workflow.

## From idea to delivery

```mermaid
flowchart LR
  Idea([An idea]) --> Sketch[Make it clear]
  Sketch --> Review{Ready to share?}
  Review -->|A little more work| Sketch
  Review -->|Yes| Build[Bring it to life]
  Build --> Deliver([Share with the world])
  classDef decision fill:#F5DDC5,stroke:#9B7052,stroke-width:1.5px
  classDef finish fill:#DFE9DA,stroke:#6F8368,stroke-width:1.5px
  class Review decision
  class Deliver finish
```

## A conversation between parts

```mermaid
sequenceDiagram
  autonumber
  actor Reader
  participant Workspace
  participant Renderer
  Reader->>Workspace: Open a diagram
  Workspace->>Renderer: Render this revision
  Renderer-->>Workspace: Valid preview
  Workspace-->>Reader: A clearer picture
```

## Keep the important things

```mermaid
stateDiagram-v2
  [*] --> Reading
  Reading --> Editing: Open source
  Editing --> Previewing: Change a line
  Previewing --> Editing: Keep exploring
  Editing --> Saved: Save
  Saved --> Reading: Return to the canvas
```
