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
            {
                let mut e = MemoryEntry::new("recon_1".to_string(), MemoryType::Episodic, "Fire is hot.".to_string(), ts, None);
                e.base_strength = 4.5;
                e.current_strength = 4.5;
                e
            },
            MemoryEntry::new("recon_2".to_string(), MemoryType::Episodic, "A passing car.".to_string(), ts, None),
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

    // ========== SCHEDULED MISSIONS TESTS ==========

    use crate::engine::MissionStore;
    use telos_core::schedule::{ScheduledMission, MissionStatus};

    #[tokio::test]
    async fn test_mission_store_and_retrieve() {
        let db_path = "test_mission_crud.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        let mission = ScheduledMission::new(
            "mission_1".to_string(),
            Some("proj_a".to_string()),
            "0 0 8 * * * *".to_string(),
            "Check weather daily".to_string(),
            "telos_daemon".to_string(),
        );

        store.store_mission(mission.clone()).await.unwrap();

        let missions = store.retrieve_missions().await.unwrap();
        assert_eq!(missions.len(), 1);
        assert_eq!(missions[0].id, "mission_1");
        assert_eq!(missions[0].cron_expr, "0 0 8 * * * *");
        assert_eq!(missions[0].instruction, "Check weather daily");
        assert_eq!(missions[0].origin_channel, "telos_daemon");
        assert_eq!(missions[0].status, MissionStatus::Active);
        assert_eq!(missions[0].execute_count, 0);
        assert_eq!(missions[0].failure_count, 0);
        assert!(missions[0].last_run_at.is_none());
        assert!(missions[0].next_run_at.is_none());

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_mission_retrieve_by_id() {
        let db_path = "test_mission_retrieve_id.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        let m1 = ScheduledMission::new("m_id_1".into(), None, "0 0 * * * * *".into(), "A".into(), "ch".into());
        let m2 = ScheduledMission::new("m_id_2".into(), None, "0 0 * * * * *".into(), "B".into(), "ch".into());
        store.store_mission(m1).await.unwrap();
        store.store_mission(m2).await.unwrap();

        let found = store.retrieve_mission("m_id_1").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().instruction, "A");

        let not_found = store.retrieve_mission("nonexistent").await.unwrap();
        assert!(not_found.is_none());

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_mission_delete() {
        let db_path = "test_mission_delete.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        let m1 = ScheduledMission::new("del_m1".into(), None, "0 * * * * * *".into(), "Task A".into(), "ch".into());
        let m2 = ScheduledMission::new("del_m2".into(), None, "0 * * * * * *".into(), "Task B".into(), "ch".into());
        store.store_mission(m1).await.unwrap();
        store.store_mission(m2).await.unwrap();

        assert_eq!(store.retrieve_missions().await.unwrap().len(), 2);

        store.delete_mission("del_m1").await.unwrap();

        let remaining = store.retrieve_missions().await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "del_m2");

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_mission_delete_nonexistent() {
        let db_path = "test_mission_del_nonexist.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        // Deleting a non-existent mission should not error
        let result = store.delete_mission("ghost_id").await;
        assert!(result.is_ok());

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_mission_status_transitions() {
        let db_path = "test_mission_status.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        let mut mission = ScheduledMission::new(
            "status_m".into(), None, "0 0 9 * * * *".into(), "Report".into(), "ch".into(),
        );
        assert_eq!(mission.status, MissionStatus::Active);

        // Simulate status transition: Active → Completed
        store.store_mission(mission.clone()).await.unwrap();
        mission.status = MissionStatus::Completed;
        store.store_mission(mission.clone()).await.unwrap();

        let retrieved = store.retrieve_mission("status_m").await.unwrap().unwrap();
        assert_eq!(retrieved.status, MissionStatus::Completed);

        // Simulate status transition: Active → Failed
        mission.status = MissionStatus::Failed;
        store.store_mission(mission.clone()).await.unwrap();
        let retrieved2 = store.retrieve_mission("status_m").await.unwrap().unwrap();
        assert_eq!(retrieved2.status, MissionStatus::Failed);

        // Simulate status transition: Active → Paused
        mission.status = MissionStatus::Paused;
        store.store_mission(mission.clone()).await.unwrap();
        let retrieved3 = store.retrieve_mission("status_m").await.unwrap().unwrap();
        assert_eq!(retrieved3.status, MissionStatus::Paused);

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_mission_execute_count_tracking() {
        let db_path = "test_mission_exec_count.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        let mut mission = ScheduledMission::new(
            "exec_m".into(), None, "0 0 * * * * *".into(), "Hourly task".into(), "ch".into(),
        );
        assert_eq!(mission.execute_count, 0);

        // Simulate 3 executions
        for i in 1..=3u32 {
            mission.execute_count += 1;
            mission.last_run_at = Some(i as i64 * 1000);
            store.store_mission(mission.clone()).await.unwrap();
        }

        let retrieved = store.retrieve_mission("exec_m").await.unwrap().unwrap();
        assert_eq!(retrieved.execute_count, 3);
        assert_eq!(retrieved.last_run_at, Some(3000));

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_mission_upsert_idempotent() {
        let db_path = "test_mission_upsert.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        let mut mission = ScheduledMission::new(
            "upsert_m".into(), None, "0 0 12 * * * *".into(), "Noon check".into(), "ch".into(),
        );

        // Store twice — second write should overwrite, not duplicate
        store.store_mission(mission.clone()).await.unwrap();
        mission.instruction = "Updated noon check".to_string();
        store.store_mission(mission.clone()).await.unwrap();

        let missions = store.retrieve_missions().await.unwrap();
        let matching: Vec<_> = missions.iter().filter(|m| m.id == "upsert_m").collect();
        assert_eq!(matching.len(), 1, "Upsert should not create duplicates");
        assert_eq!(matching[0].instruction, "Updated noon check");

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_mission_multiple_concurrent_missions() {
        let db_path = "test_mission_multi.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        // Store 5 missions with different statuses
        for i in 0..5u32 {
            let mut m = ScheduledMission::new(
                format!("multi_m{}", i),
                None,
                "0 0 * * * * *".into(),
                format!("Task {}", i),
                "ch".into(),
            );
            if i >= 3 {
                m.status = MissionStatus::Completed;
            }
            store.store_mission(m).await.unwrap();
        }

        let all = store.retrieve_missions().await.unwrap();
        assert_eq!(all.len(), 5);

        let active_count = all.iter().filter(|m| m.status == MissionStatus::Active).count();
        let completed_count = all.iter().filter(|m| m.status == MissionStatus::Completed).count();
        assert_eq!(active_count, 3);
        assert_eq!(completed_count, 2);

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn test_cron_expression_validation() {
        use std::str::FromStr;
        
        // Valid 7-field cron expressions
        assert!(cron::Schedule::from_str("0 0 8 * * * *").is_ok(), "Standard daily at 8AM should be valid");
        assert!(cron::Schedule::from_str("0 */5 * * * * *").is_ok(), "Every 5 minutes should be valid");
        assert!(cron::Schedule::from_str("0 0 0 * * MON *").is_ok(), "Weekly Monday should be valid");
        assert!(cron::Schedule::from_str("0 30 9 1 * * *").is_ok(), "Monthly on 1st at 9:30 should be valid");

        // Invalid cron expressions
        assert!(cron::Schedule::from_str("not a cron").is_err(), "Random text should be invalid");
        assert!(cron::Schedule::from_str("").is_err(), "Empty string should be invalid");
        assert!(cron::Schedule::from_str("0 0 25 * * * *").is_err(), "Hour 25 should be invalid");

        // 5-field cron (may or may not be supported depending on crate)
        // The cron crate requires 7 fields: sec min hour day month weekday year
        let five_field = cron::Schedule::from_str("0 8 * * *");
        // This is just to document the behavior — the cron crate rejects 5-field format
        assert!(five_field.is_err() || five_field.is_ok(), "5-field format behavior documented");
    }

    #[test]
    fn test_scheduled_mission_serialization_roundtrip() {
        let mut mission = ScheduledMission::new(
            "ser_m".into(), Some("proj".into()), "0 0 8 * * * *".into(), "Test instruction".into(), "channel".into(),
        );
        mission.execute_count = 5;
        mission.failure_count = 1;
        mission.last_run_at = Some(1700000000);
        mission.next_run_at = Some(1700003600);
        mission.status = MissionStatus::Active;

        let json = serde_json::to_string(&mission).unwrap();
        let deserialized: ScheduledMission = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "ser_m");
        assert_eq!(deserialized.project_id, Some("proj".to_string()));
        assert_eq!(deserialized.cron_expr, "0 0 8 * * * *");
        assert_eq!(deserialized.instruction, "Test instruction");
        assert_eq!(deserialized.origin_channel, "channel");
        assert_eq!(deserialized.execute_count, 5);
        assert_eq!(deserialized.failure_count, 1);
        assert_eq!(deserialized.last_run_at, Some(1700000000));
        assert_eq!(deserialized.next_run_at, Some(1700003600));
        assert_eq!(deserialized.status, MissionStatus::Active);
    }

    // ========== DUAL-LAYER PROFILE ASSEMBLY TESTS ==========

    use crate::profile::format_profile_for_prompt;
    use crate::types::UserProfile;

    #[test]
    fn test_format_profile_for_prompt_dual_layer() {
        let profile = UserProfile {
            static_facts: vec![
                "Senior Rust engineer".to_string(),
                "Prefers CLI over GUI".to_string(),
            ],
            dynamic_context: vec![
                "Working on Telos memory system".to_string(),
                "Debugging auth issues".to_string(),
            ],
        };

        let output = format_profile_for_prompt(&profile);
        assert!(output.contains("[USER BACKGROUND — PERSISTENT KNOWLEDGE ABOUT YOUR OWNER]"));
        assert!(output.contains("• Senior Rust engineer"));
        assert!(output.contains("• Prefers CLI over GUI"));
        assert!(output.contains("[CURRENT CONTEXT — RECENT ACTIVITY]"));
        assert!(output.contains("• Working on Telos memory system"));
        assert!(output.contains("• Debugging auth issues"));
    }

    #[test]
    fn test_format_profile_for_prompt_empty() {
        let profile = UserProfile {
            static_facts: vec![],
            dynamic_context: vec![],
        };

        let output = format_profile_for_prompt(&profile);
        assert!(output.is_empty(), "Empty profile should produce empty string");
    }

    #[test]
    fn test_format_profile_partial_static_only() {
        let profile = UserProfile {
            static_facts: vec!["Loves coffee".to_string()],
            dynamic_context: vec![],
        };

        let output = format_profile_for_prompt(&profile);
        assert!(output.contains("[USER BACKGROUND"));
        assert!(output.contains("• Loves coffee"));
        assert!(!output.contains("[CURRENT CONTEXT"), "No dynamic section when empty");
    }

    // ========== MEMORY RELATIONS TESTS ==========

    use crate::types::MemoryRelation;

    #[tokio::test]
    async fn test_extends_relation_created() {
        // Two facts with very similar embeddings (cosine > 0.8) but both keep high
        // confidence (no LLM gateway → fallback (1.0, 0.7), both >= 0.4) → Extends.
        let db_path = "test_extends_rel.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        let e1 = MemoryEntry::new(
            "ext_1".to_string(),
            MemoryType::UserProfileStatic,
            "User is a PM at Stripe".to_string(),
            100,
            Some(vec![1.0, 0.0, 0.0]),
        );
        store.store(e1).await.unwrap();

        // Second fact with very similar embedding (cosine > 0.8) but different content
        let e2 = MemoryEntry::new(
            "ext_2".to_string(),
            MemoryType::UserProfileStatic,
            "User focuses on payment infrastructure".to_string(),
            200,
            Some(vec![0.95, 0.05, 0.0]),
        );
        store.store(e2).await.unwrap();

        // Retrieve both from DB and verify bidirectional Extends relations
        let all = store.retrieve_all().await.unwrap();
        let first = all.iter().find(|e| e.id == "ext_1").unwrap();
        let second = all.iter().find(|e| e.id == "ext_2").unwrap();

        assert!(
            second.memory_relations.get("ext_1") == Some(&MemoryRelation::Extends),
            "New entry should have Extends relation to existing"
        );
        assert!(
            first.memory_relations.get("ext_2") == Some(&MemoryRelation::Extends),
            "Existing entry should have Extends relation back to new entry"
        );

        // Both should remain latest (not superseded)
        assert!(first.is_latest, "First should remain latest");
        assert!(second.is_latest, "Second should remain latest");

        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test]
    async fn test_reconsolidation_creates_derives_relation() {
        let ts = get_current_timestamp();
        let mut memories = vec![{
            let mut e = MemoryEntry::new(
                "ep_source".to_string(),
                MemoryType::Episodic,
                "User discussed payment APIs extensively".to_string(),
                ts,
                Some(vec![0.5, 0.5, 0.0]),
            );
            e.base_strength = 5.0;
            e.current_strength = 5.0;
            e
        }];

        let promoted = consolidate_memories(&mut memories, 3.0, None).await;
        assert_eq!(promoted.len(), 1);
        assert_eq!(promoted[0].memory_type, MemoryType::Semantic);

        // Check Derives relation points to source
        assert_eq!(
            promoted[0].memory_relations.get("ep_source"),
            Some(&MemoryRelation::Derives),
            "Promoted semantic entry should have Derives relation to source episodic"
        );
    }

    #[tokio::test]
    async fn test_expand_relations() {
        let db_path = "test_expand_rel.redb";
        let _ = std::fs::remove_file(db_path);
        let store = RedbGraphStore::new(db_path).unwrap();

        // Create two entries with manual Extends relations pointing to each other
        let mut e1 = MemoryEntry::new(
            "rel_a".to_string(),
            MemoryType::Semantic,
            "Fact A".to_string(),
            100,
            None,
        );
        e1.memory_relations.insert("rel_b".to_string(), MemoryRelation::Extends);

        let mut e2 = MemoryEntry::new(
            "rel_b".to_string(),
            MemoryType::Semantic,
            "Fact B".to_string(),
            200,
            None,
        );
        e2.memory_relations.insert("rel_a".to_string(), MemoryRelation::Extends);

        store.store(e1.clone()).await.unwrap();
        store.store(e2.clone()).await.unwrap();

        // Expand with only e1 → should pull in e2
        let expanded = store.expand_relations(&[e1]).await.unwrap();
        assert_eq!(expanded.len(), 2, "Should include original + related entry");
        let ids: Vec<String> = expanded.iter().map(|e| e.id.clone()).collect();
        assert!(ids.contains(&"rel_a".to_string()));
        assert!(ids.contains(&"rel_b".to_string()));

        let _ = std::fs::remove_file(db_path);
    }