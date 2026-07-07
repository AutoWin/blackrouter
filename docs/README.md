# BlackRouter Documentation

Welcome to the BlackRouter documentation! This directory contains all project documentation.

## 📚 Documents

### [Request Processing Pipeline](./REQUEST_PROCESSING_PIPELINE.md)
Luồng xử lý request từ lúc nhận prompt đến khi trả về output.

**Key sections:**
- Kiến trúc tổng quan
- 11 bước xử lý chi tiết
- Request/Response Translation
- Rate Limiting
- Usage Recording
- Sequence Diagram
- Ví dụ cụ thể

---

### [Provider Login Mechanisms](./PROVIDER_LOGIN_MECHANISMS.md)
Các cơ chế xác thực của providers - API Key, OAuth 2.0, và No Auth.

**Key sections:**
- API Key Authentication
- OAuth 2.0 Flows (Device Code, Authorization Code, PKCE)
- Supported Providers
- API Reference

---

### [Implementation Status](./IMPLEMENTATION_STATUS.md)
Current state of the project - what's completed, in progress, and planned.

**Key sections:**
- Completed features
- Test coverage
- Metrics and statistics

---

### [Development Plan](./DEVELOPMENT_PLAN.md)
Comprehensive roadmap for future development.

**Key sections:**
- Phase 1: Core Streaming & Usage
- Phase 2: Provider Expansion
- Phase 3: Production Readiness
- Phase 4: Advanced Features
- Technical specifications
- Timeline estimates

---

## 🚀 Quick Start

1. **Build the project:**
   ```bash
   cargo build
   ```

2. **Run tests:**
   ```bash
   cargo test
   ```

3. **Start the server:**
   ```bash
   cargo run -p blackrouter-bin
   ```

4. **Check health:**
   ```bash
   curl http://localhost:20130/health
   ```

---

## 📖 Additional Resources

- **README:** [../README.md](../README.md) - Project overview and setup
- **Source Code:** [../crates/](../crates/) - Rust workspace crates
- **Configuration:** [../.env.example](../.env.example) - Environment variables

---

## 🤝 Contributing

See the [Development Plan](./DEVELOPMENT_PLAN.md) for current priorities and tasks.

### Development Setup

1. Install Rust (see [rust-toolchain.toml](../rust-toolchain.toml))
2. Clone the repository
3. Copy `.env.example` to `.env`
4. Run `cargo build`
5. Run `cargo test`

### Code Structure

```
blackrouter/
├── crates/
│   ├── blackrouter-api/        # HTTP API (Axum)
│   ├── blackrouter-bin/        # Binary entry point
│   ├── blackrouter-common/     # Shared utilities
│   ├── blackrouter-config/     # Configuration
│   ├── blackrouter-core/       # Core routing logic
│   ├── blackrouter-providers/  # Provider abstractions
│   ├── blackrouter-rtk/        # Rate limiting & metrics
│   ├── blackrouter-storage/    # SQLite storage
│   ├── blackrouter-telegram/   # Telegram bot
│   └── blackrouter-translator/ # Wire format translation
├── docs/                       # This directory
├── Cargo.toml                  # Workspace config
├── Dockerfile
└── docker-compose.yml
```

---

## 📝 License

MIT License - See [LICENSE](../LICENSE)
