use crate::StorageError;
use redb::{Database, ReadableTable, TableDefinition};
use std::path::Path;

// Since redb requires keys and values to implement the `Value` trait,
// we typically serialize our state into JSON (or bincode/MessagePack) and store it as a string or byte array.
// For simplicity in this mock, we'll store JSON strings.

const GRAPH_STATE_TABLE: TableDefinition<&str, &str> = TableDefinition::new("graph_state");

pub struct CheckpointManager {
    db: Database,
}

impl CheckpointManager {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        let db = Database::create(path).map_err(|_| StorageError::IoError)?;
        Ok(Self { db })
    }

    pub fn save_checkpoint(&self, graph_id: &str, state_json: &str) -> Result<(), StorageError> {
        let write_txn = self.db.begin_write().map_err(|_| StorageError::IoError)?;
        {
            let mut table = write_txn
                .open_table(GRAPH_STATE_TABLE)
                .map_err(|_| StorageError::IoError)?;
            table
                .insert(graph_id, state_json)
                .map_err(|_| StorageError::IoError)?;
        }
        write_txn.commit().map_err(|_| StorageError::IoError)?;
        Ok(())
    }

    pub fn delete_checkpoint(&self, graph_id: &str) -> Result<(), StorageError> {
        let write_txn = self.db.begin_write().map_err(|_| StorageError::IoError)?;
        {
            let mut table = write_txn
                .open_table(GRAPH_STATE_TABLE)
                .map_err(|_| StorageError::IoError)?;
            table.remove(graph_id).map_err(|_| StorageError::IoError)?;
        }
        write_txn.commit().map_err(|_| StorageError::IoError)?;
        Ok(())
    }

    pub fn restore_checkpoint(&self, graph_id: &str) -> Result<Option<String>, StorageError> {
        let read_txn = self.db.begin_read().map_err(|_| StorageError::IoError)?;
        let table = read_txn
            .open_table(GRAPH_STATE_TABLE)
            .map_err(|_| StorageError::IoError)?;

        let value = table.get(graph_id).map_err(|_| StorageError::IoError)?;
        if let Some(v) = value {
            Ok(Some(v.value().to_string()))
        } else {
            Ok(None)
        }
    }

    /// Retrieves all saved checkpoints from the database.
    pub fn get_all_checkpoints(&self) -> Result<Vec<(String, String)>, StorageError> {
        let read_txn = self.db.begin_read().map_err(|_| StorageError::IoError)?;
        let table = read_txn
            .open_table(GRAPH_STATE_TABLE)
            .map_err(|_| StorageError::IoError)?;

        let mut all_checkpoints = Vec::new();
        let iter = table.iter().map_err(|_| StorageError::IoError)?;
        for result in iter {
            let (key, value) = result.map_err(|_| StorageError::IoError)?;
            all_checkpoints.push((key.value().to_string(), value.value().to_string()));
        }
        Ok(all_checkpoints)
    }
}
