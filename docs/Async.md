# luars Async Documentation

This directory contains the complete documentation for luars async features.

## Documentation Index

| Document | Contents |
|----------|----------|
| [Getting Started](async/01-getting-started.md) | 5-minute async quickstart, your first async function |
| [API Reference](async/02-api-reference.md) | Detailed description of all async types and methods |
| [Examples](async/03-examples.md) | Code examples from simple to complex |
| [Internal Architecture](async/04-architecture.md) | Coroutine↔Future bridging implementation details |
| [Multi-VM Patterns](async/05-multi-vm.md) | Design patterns for multi-LuaVM concurrent processing |
| [HTTP Server Example](async/06-http-server.md) | Complete async HTTP server walkthrough |

## Design Document

For lower-level design decisions and industry research, see [Async_Design.md](../Async_Design.md).

## Core Concept Overview

```text
Rust async runtime (tokio)
  └── AsyncThread::poll()          ← Driver: implements Future trait
        ├── has pending future? → poll it
        │     ├── Pending → return Poll::Pending
        │     └── Ready(result) → resume(result) → continue checking
        └── no pending future → resume(args)
              ├── coroutine finished → return Poll::Ready
              ├── async yield (sentinel) → take future, poll it
              └── normal yield → wake & return Pending
```

**Key point**: From Lua's perspective, async functions behave exactly like normal synchronous functions. The async yield/resume is completely transparent to Lua code.
