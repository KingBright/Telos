# Module 4 Tasks: Hierarchical Memory OS

- [x] Define `MemoryType` and `MemoryQuery`.
- [x] Implement `MemoryOS` trait using `lance` (for vector) and `redb` (for semantic graph).
- [x] Implement Ebbinghaus forgetting curve worker.
- [x] Add memory reconsolidation logic.
- [x] Write tests ensuring memory decay works and limits storage explosion.

## Notes/Issues
- Implemented `MemoryOS` trait using `redb` as the core storage engine. Mapped vectors natively or using built-in embedding cosine similarity to avoid heavy external database dependencies initially, prioritizing zero-network overhead.
- Added a worker implementation `decay.rs` that applies a logarithmic decay factor mapping to Ebbinghaus forgetting curve to naturally prune episodic memories.
- Implemented `reconsolidation.rs` to promote frequently accessed episodic memories into semantic knowledge.
- Wrote unit tests confirming storage, retrieval, decay formulas, and reconsolidation features.