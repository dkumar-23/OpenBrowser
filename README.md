# OpenBrowser

A modular, high-performance browser runtime written in Rust — designed for embedded use, automation, and agentic web interaction.

## Features Implemented

### Core Runtimes
| Crate | Status | Description |
|-------|--------|-------------|
| `runtime-core` | ✅ | Core browser engine abstractions and interfaces |
| `runtime-dom` | ✅ | DOM manipulation and tree management |
| `runtime-js` | ✅ | JavaScript engine integration (V8 via `v8_handler`) |
| `runtime-browser` | ✅ | High-level browser orchestration |
| `runtime-cli` | ✅ | Command-line interface for browser control |

### Network & Protocol
| Crate | Status | Description |
|-------|--------|-------------|
| `runtime-network` | ✅ | HTTP/HTTPS request handling with dynamic User-Agent rotation |
| `runtime-adapters-http` | ✅ | Pluggable HTTP adapter system |

### Security & Policy
| Crate | Status | Description |
|-------|--------|-------------|
| `runtime-auth` | ✅ | Authentication framework (OAuth, session, token management) |
| `runtime-policy` | ✅ | Security policy enforcement and sandbox rules |

### Observability
| Crate | Status | Description |
|-------|--------|-------------|
| `runtime-observability` | ✅ | Metrics, tracing, and logging infrastructure |

### Agentic Capabilities
| Crate | Status | Description |
|-------|--------|-------------|
| `runtime-agent` | ✅ | Agent framework for autonomous web navigation |
| `runtime-interaction` | ✅ | User interaction simulation (clicks, scrolls, form fills) |

### Extensions
| Crate | Status | Description |
|-------|--------|-------------|
| `runtime-mcp` | ✅ | MCP (Model Context Protocol) integration for LLM tooling |

### Sandboxing
| Crate | Status | Description |
|-------|--------|-------------|
| `runtime-sandbox` | ✅ | Secure execution sandboxing |

## Features Planned

- [ ] WebDriver compatibility layer
- [ ] Headless/headful mode toggle
- [ ] Tab/window management
- [ ] Cookie jar persistence
- [ ] Service worker support
- [ ] WebExtensions API
- [ ] Screenshot capture
- [ ] PDF generation
- [ ] WebSocket support in runtime-network
- [ ] HTTP/2 and HTTP/3 support
- [ ] Resource interceptor/rewriter
- [ ] Geolocation mocking
- [ ] Device emulation profiles

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      runtime-cli                        │
└─────────────────────────────────────────────────────────┘
                            │
┌─────────────────────────────────────────────────────────┐
│                    runtime-browser                      │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────────┐  │
│  │runtime-dom│  │runtime-js│  │  runtime-interaction │  │
│  └──────────┘  └──────────┘  └──────────────────────┘  │
└─────────────────────────────────────────────────────────┘
                            │
┌─────────────────────────────────────────────────────────┐
│                   runtime-network                       │
│  ┌──────────────────────┐  ┌───────────────────────┐  │
│  │runtime-adapters-http │  │    runtime-sandbox    │  │
│  └──────────────────────┘  └───────────────────────┘  │
└─────────────────────────────────────────────────────────┘
                            │
         ┌──────────────────┼──────────────────┐
         ▼                  ▼                  ▼
  ┌────────────┐    ┌────────────┐    ┌────────────┐
  │runtime-auth│    │runtime-policy│  │runtime-obs │
  └────────────┘    └────────────┘    └────────────┘
                            │
                    ┌────────────┐
                    │runtime-agent│
                    │  + runtime-mcp │
                    └────────────┘
```

## Getting Started

```bash
# Build all crates
cargo build --workspace

# Run the CLI
cargo run -p runtime-cli -- --help

# Run tests
cargo test --workspace
```

## Dependencies

- Rust 1.75+
- Tokio (async runtime)
- V8 (JavaScript engine via v8_handler)
- reqwest (HTTP client)

## License

MIT OR Apache-2.0
