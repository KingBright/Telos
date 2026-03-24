use std::sync::Arc;
use tokio::time::Duration;
use std::str::FromStr;
use tracing::{error, info, debug};
use uuid::Uuid;

use telos_core::schedule::{ScheduledMission, MissionStatus};
use telos_memory::engine::{MissionStore, RedbGraphStore};
use telos_hci::{AgentEvent, EventBroker};

pub struct SchedulerActor {
    memory_os: Arc<RedbGraphStore>,
    broker: Arc<dyn EventBroker>,
}

impl SchedulerActor {
    pub fn new(memory_os: Arc<RedbGraphStore>, broker: Arc<dyn EventBroker>) -> Self {
        Self { memory_os, broker }
    }

    pub async fn run(&self) {
        info!("[SchedulerActor] Starting up...");
        
        // 1. Catchup missed missions
        self.catchup_missed_missions().await;

        let memory_os_clone = self.memory_os.clone();
        let broker_clone = self.broker.clone();
        
        info!("[SchedulerActor] Running via polling engine.");
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                Self::poll_and_dispatch(&memory_os_clone, &broker_clone).await;
            }
        });
    }

    async fn catchup_missed_missions(&self) {
        // Load missions from DB
        if let Ok(missions) = self.memory_os.retrieve_missions().await {
            let now = chrono::Utc::now().timestamp();
            for mut mission in missions {
                if mission.status == MissionStatus::Active {
                    // If we missed a run
                    if let Some(next_run) = mission.next_run_at {
                        if now >= next_run {
                            info!("[SchedulerActor] Catching up missed mission: {}", mission.id);
                            self.dispatch_mission(&mission).await;
                            mission.execute_count += 1;
                            
                            // Calculate next run
                            if let Ok(schedule) = cron::Schedule::from_str(mission.cron_expr.as_str()) {
                                if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                                    mission.last_run_at = Some(now);
                                    mission.next_run_at = Some(next.timestamp());
                                    let _ = self.memory_os.store_mission(mission).await;
                                }
                            }
                        }
                    } else {
                        // First time scheduling
                        if let Ok(schedule) = cron::Schedule::from_str(mission.cron_expr.as_str()) {
                            if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                                mission.next_run_at = Some(next.timestamp());
                                let _ = self.memory_os.store_mission(mission).await;
                            }
                        }
                    }
                }
            }
        }
    }

    async fn poll_and_dispatch(memory_os: &Arc<RedbGraphStore>, broker: &Arc<dyn EventBroker>) {
        if let Ok(missions) = memory_os.retrieve_missions().await {
            let now = chrono::Utc::now().timestamp();
            for mut mission in missions {
                if mission.status == MissionStatus::Active {
                    // Parse cron to ensure next_run_at is populated
                    let schedule = match cron::Schedule::from_str(mission.cron_expr.as_str()) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("[SchedulerActor] Invalid cron expression for mission {}: {}", mission.id, e);
                            mission.status = MissionStatus::Failed;
                            let _ = memory_os.store_mission(mission).await;
                            continue;
                        }
                    };

                    let next_run = mission.next_run_at.unwrap_or_else(|| {
                        schedule.upcoming(chrono::Utc).next().map(|dt| dt.timestamp()).unwrap_or(now + 86400)
                    });

                    if now >= next_run {
                        debug!("[SchedulerActor] Triggering scheduled mission: {}", mission.id);
                        
                        // Fire event
                        let event = AgentEvent::SystemMission {
                            mission_id: mission.id.clone(),
                            context: mission.instruction.clone(),
                            origin_channel: mission.origin_channel.clone(),
                            trace_id: Uuid::new_v4(),
                        };
                        
                        if let Err(e) = broker.publish_event(event).await {
                            error!("[SchedulerActor] Failed to dispatch mission {}: {:?}", mission.id, e);
                        } else {
                            mission.execute_count += 1;
                        }

                        // Set next run
                        mission.last_run_at = Some(now);
                        if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                            mission.next_run_at = Some(next.timestamp());
                        }
                        
                        let _ = memory_os.store_mission(mission).await;
                    } else if mission.next_run_at.is_none() {
                        mission.next_run_at = Some(next_run);
                        let _ = memory_os.store_mission(mission).await;
                    }
                }
            }
        }
    }

    async fn dispatch_mission(&self, mission: &ScheduledMission) {
        let event = AgentEvent::SystemMission {
            mission_id: mission.id.clone(),
            context: mission.instruction.clone(),
            origin_channel: mission.origin_channel.clone(),
            trace_id: Uuid::new_v4(),
        };
        let _ = self.broker.publish_event(event).await;
    }
}
