#[cfg(test)]
mod tests {
    use crate::types::{MemoryEntry, MemoryQuery, MemoryType};
    use crate::engine::{MemoryOS, RedbGraphStore};
    use crate::decay::{apply_decay, get_current_timestamp};
    use crate::reconsolidation::consolidate_memories;

    #[tokio::test]
    async fn test_store_and_retrieve_semantic() {
        let db_path = "test_semantic_db.redb";
        let _ = std::fs::remove_file(db_path); // Ensure clean start
        let store = RedbGraphStore::new(db_path).unwrap();

        let entry = MemoryEntry::new(
            "mem_1".to_string(),
            MemoryType::Semantic,
            "Rust is a programming language.".to_string(),
            get_current_timestamp(),
            None,
        );

        store.store(entry).await.unwrap();

        let results = store.retrieve(MemoryQuery::EntityLookup { entity: "Rust".to_string() }).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "Rust is a programming language.");

        let _ = std::fs::remove_file(db_path); // Cleanup
    }

    #[tokio::test]
    async fn test_vector_search() {
         let db_path = "test_vector_db.redb";
         let _ = std::fs::remove_file(db_path); // Ensure clean start
         let store = RedbGraphStore::new(db_path).unwrap();

         let entry1 = MemoryEntry::new(
             "vec_1".to_string(),
             MemoryType::Episodic,
             "I like apples.".to_string(),
             get_current_timestamp(),
             Some(vec![1.0, 0.0, 0.0]),
         );

         let entry2 = MemoryEntry::new(
             "vec_2".to_string(),
             MemoryType::Episodic,
             "I like bananas.".to_string(),
             get_current_timestamp(),
             Some(vec![0.0, 1.0, 0.0]),
         );

         store.store(entry1).await.unwrap();
         store.store(entry2).await.unwrap();

         // Search for something close to entry1
         let query_vec = vec![0.9, 0.1, 0.0];
         let results = store.retrieve(MemoryQuery::VectorSearch { query: query_vec, top_k: 1 }).await.unwrap();

         assert_eq!(results.len(), 1);
         assert_eq!(results[0].id, "vec_1"); // Because dot product is highest

         let _ = std::fs::remove_file(db_path); // Cleanup
    }

    #[test]
    fn test_memory_decay() {
        let ts = get_current_timestamp();
        let mut entry = MemoryEntry::new(
            "decay_1".to_string(),
            MemoryType::Episodic,
            "A temporary thought.".to_string(),
            ts - 86400, // 24 hours ago
            None,
        );

        // Initial strength is 1.0. Let's see if it gets pruned at min_strength 0.5
        let pruned = apply_decay(&mut entry, ts, 0.5);

        // Decay factor for 24h is e^(-1) ~ 0.36
        // Current strength becomes 1.0 * 0.36 = 0.36
        // Since 0.36 < 0.5, it should be pruned.
        assert!(pruned);
        assert!(entry.current_strength < 0.5);
    }

    #[tokio::test]
    async fn test_memory_reconsolidation() {
        let ts = get_current_timestamp();
        let mut memories = vec![
            MemoryEntry {
                id: "recon_1".to_string(),
                memory_type: MemoryType::Episodic,
                content: "Fire is hot.".to_string(),
                base_strength: 4.5,
                current_strength: 4.5,
                created_at: ts,
                last_accessed: ts,
                embedding: None,
            },
            MemoryEntry {
                id: "recon_2".to_string(),
                memory_type: MemoryType::Episodic,
                content: "A passing car.".to_string(),
                base_strength: 1.0,
                current_strength: 1.0,
                created_at: ts,
                last_accessed: ts,
                embedding: None,
            }
        ];

        let new_semantics = consolidate_memories(&mut memories, 4.0, None).await;

        assert_eq!(new_semantics.len(), 1);
        assert_eq!(new_semantics[0].memory_type, MemoryType::Semantic);
        assert_eq!(new_semantics[0].content, "Fire is hot.");
    }
}
