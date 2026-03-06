# Module 8 Tasks: Zero-Trust Security & Vault

- [x] Implement `SecurityVault` traits.
- [x] Build temporary token generation with tight TTL logic.
- [x] Add ABAC check for MCP tool invocations.
- [x] Test rejection on out-of-bounds agent operations.

## Notes/Issues
- Utilized `casbin` with memory adapter to implement Attribute-Based Access Control (ABAC) with models and policies directly instantiated from text.
- Integrated `jsonwebtoken` for tight TTL tokens simulated using `lease_temporary_credential`.
- Written and passed comprehensive unit tests covering policy rejection, lease success, and token expiration logic. Added required `rust_crypto` feature to `jsonwebtoken` for proper CryptoProvider setup during JWT signing.
