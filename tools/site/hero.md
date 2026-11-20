# Payments service

The diagrams in this file are drawn on the Tidemap landing page by the real engine.
`python tools/site/build.py` exports them again after an engine change.

## Request path

```mermaid
flowchart LR
  Client([Client]) --> Gateway[API gateway]
  Gateway --> Auth{Token valid?}
  Auth -->|No| Reject([401])
  Auth -->|Yes| Charges[Charges API]
  Charges --> Ledger[(Ledger)]
  Charges --> Events[[Event bus]]
```

## Checkout

```mermaid
sequenceDiagram
  actor Shopper
  participant App
  participant API as Charges API
  Shopper->>App: Pay
  App->>API: POST /charges
  API-->>App: 201 Created
  App-->>Shopper: Receipt
```

## Charge lifecycle

```mermaid
stateDiagram-v2
  direction LR
  [*] --> Pending
  Pending --> Paid: card ok
  Pending --> Failed: declined
  Paid --> Refunded: refund
  Paid --> [*]
  Failed --> [*]
  Refunded --> [*]
```
