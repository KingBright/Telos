#[cfg(test)]
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
                access_count: 0,
                confidence: 1.0,
                similarity_score: None,
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
                access_count: 0,
                confidence: 1.0,
                similarity_score: None,
            }
        ];

        let new_semantics = consolidate_memories(&mut memories, 4.0, None).await;

        assert_eq!(new_semantics.len(), 1);
        assert_eq!(new_semantics[0].memory_type, MemoryType::Semantic);
        assert_eq!(new_semantics[0].content, "Fire is hot.");
    }

    #[tokio::test]
    async fn test_ingest_user_feedback() {
        use crate::integration::MemoryIntegration;

        let db_path = "test_feedback_db.redb";
        let _ = std::fs::remove_file(db_path); // Ensure clean start
        let store = RedbGraphStore::new(db_path).unwrap();

        let feedback = "Never delete system files.";
        let strength = 5.0;

        store.ingest_user_feedback(feedback, strength).await.unwrap();

        // Use a broader query and manually filter to find the specific feedback entry
        let results = store.retrieve(MemoryQuery::EntityLookup { entity: "Never".to_string() }).await.unwrap();
        assert_eq!(results.len(), 1);

        let entry = &results[0];
        assert_eq!(entry.content, feedback);
        assert_eq!(entry.memory_type, MemoryType::Semantic);
        assert_eq!(entry.base_strength, strength);
        assert_eq!(entry.current_strength, strength);

        let _ = std::fs::remove_file(db_path); // Cleanup
    }

    // --- NEW TESTS: Memory OS 100% coverage ---

    #[test]
    fn test_decay_interaction_event() {
        // InteractionEvent should decay with 48h half-life (slower than Episodic 24h)
        let ts = get_current_timestamp();
        let mut entry = MemoryEntry::new(
            "ie_1".to_string(),
            MemoryType::InteractionEvent,
            "User asked about weather".to_string(),
            ts - 86400, // 24 hours ago
            None,
        );

        let pruned = apply_decay(&mut entry, ts, 0.5);
        // At 24h with 48h half-life: e^(-24/48) = e^(-0.5) ≈ 0.607
        // So strength = 1.0 * 0.607 ≈ 0.607, should NOT be pruned
        assert!(!pruned, "InteractionEvent should not be pruned after only 24h");
        assert!(entry.current_strength > 0.5);

        // After 96h (4 days): e^(-96/48) = e^(-2) ≈ 0.135
        let mut entry2 = MemoryEntry::new(
            "ie_2".to_string(),
            MemoryType::InteractionEvent,
            "Old conversation".to_string(),
            ts - 86400 * 4, // 4 days ago
            None,
        );
        let pruned2 = apply_decay(&mut entry2, ts, 0.5);
        assert!(pruned2, "InteractionEvent should be pruned after 4 days");
    }

    #[test]
    fn test_semantic_never_decays() {
        let ts = get_current_timestamp();
        let mut entry = MemoryEntry::new(
            "sem_1".to_string(),
            MemoryType::Semantic,
            "Permanent fact".to_string(),
            ts - 86400 * 365, // 1 year ago!
            None,
        );

        let pruned = apply_decay(&mut entry, ts, 0.5);
        assert!(!pruned, "Semantic memory should never decay");
        assert_eq!(entry.current_strength, 1.0, "Semantic strength should be unchanged");
    }

    #[tokio::test]
    async fn test_delete_and_retrieve_all() {
        let db_path = "test_delete_db.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        let e1 = MemoryEntry::new("del_1".to_string(), MemoryType::Episodic, "First memory".to_string(), 100, None);
        let e2 = MemoryEntry::new("del_2".to_string(), MemoryType::Episodic, "Second memory".to_string(), 200, None);

        store.store(e1).await.unwrap();
        store.store(e2).await.unwrap();

        let all = store.retrieve_all().await.unwrap();
        assert_eq!(all.len(), 2);

        store.delete("del_1").await.unwrap();

        let remaining = store.retrieve_all().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "del_2");

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_trigger_fade_consolidation() {
        let db_path = "test_fade_db.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        let ts = get_current_timestamp();
        // Store a very old episodic memory (should be pruned)
        let old_entry = MemoryEntry::new(
            "old_ep".to_string(),
            MemoryType::Episodic,
            "Ancient memory".to_string(),
            ts - 86400 * 30, // 30 days ago
            None,
        );
        // Store a recent episodic memory (should survive)
        let recent_entry = MemoryEntry::new(
            "recent_ep".to_string(),
            MemoryType::Episodic,
            "Recent memory".to_string(),
            ts,
            None,
        );
        // Store a semantic memory (never decays)
        let semantic_entry = MemoryEntry::new(
            "perm_sem".to_string(),
            MemoryType::Semantic,
            "Permanent fact".to_string(),
            ts - 86400 * 30,
            None,
        );

        store.store(old_entry).await.unwrap();
        store.store(recent_entry).await.unwrap();
        store.store(semantic_entry).await.unwrap();

        assert_eq!(store.retrieve_all().await.unwrap().len(), 3);

        // Run fade consolidation
        store.trigger_fade_consolidation().await.unwrap();

        let remaining = store.retrieve_all().await.unwrap();
        // Old episodic should be pruned, recent episodic & semantic should survive
        assert_eq!(remaining.len(), 2);
        let ids: Vec<String> = remaining.iter().map(|e| e.id.clone()).collect();
        assert!(ids.contains(&"recent_ep".to_string()));
        assert!(ids.contains(&"perm_sem".to_string()));

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_confidence_field_default() {
        // Test that deserializing old data without confidence works
        let json_str = r#"{"id":"test","memory_type":"Semantic","content":"hello","base_strength":1.0,"current_strength":1.0,"created_at":100,"last_accessed":100,"embedding":null}"#;
        let entry: MemoryEntry = serde_json::from_str(json_str).unwrap();
        assert_eq!(entry.confidence, 1.0, "Default confidence should be 1.0");
        assert_eq!(entry.access_count, 0, "Default access_count should be 0");
    }

    #[test]
    fn test_access_count_tracking() {
        let mut entry = MemoryEntry::new(
            "ac_1".to_string(),
            MemoryType::Episodic,
            "Test".to_string(),
            100,
            None,
        );

        assert_eq!(entry.access_count, 0);
        assert_eq!(entry.base_strength, 1.0);

        entry.access(200);
        assert_eq!(entry.access_count, 1);
        assert!(entry.base_strength > 1.0); // Should boost

        let first_strength = entry.base_strength;
        entry.access(300);
        assert_eq!(entry.access_count, 2);
        let second_boost = entry.base_strength - first_strength;

        entry.access(400);
        let third_boost = entry.base_strength - first_strength - second_boost;

        // Verify diminishing returns
        assert!(third_boost < second_boost, "Boost should diminish with more accesses");
    }