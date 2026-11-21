CREATE TABLE events (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  rig_id TEXT,
  node_id TEXT,
  type TEXT NOT NULL,
  payload TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE queue_transitions (
  transition_id INTEGER PRIMARY KEY AUTOINCREMENT,
  qitem_id TEXT NOT NULL,
  ts TEXT NOT NULL,
  state TEXT NOT NULL,
  transition_note TEXT,
  actor_session TEXT NOT NULL,
  closure_reason TEXT,
  closure_target TEXT,
  identity_provenance TEXT,
  owner_notification_kind TEXT,
  owner_notification_level TEXT,
  identity_principal TEXT
);
CREATE TABLE queue_items (
  qitem_id TEXT PRIMARY KEY,
  ts_created TEXT NOT NULL,
  ts_updated TEXT NOT NULL,
  source_session TEXT NOT NULL,
  destination_session TEXT NOT NULL,
  state TEXT NOT NULL,
  priority TEXT NOT NULL DEFAULT 'routine',
  tier TEXT,
  tags TEXT,
  blocked_on TEXT,
  handed_off_to TEXT,
  handed_off_from TEXT,
  expires_at TEXT,
  chain_of_record TEXT,
  body TEXT NOT NULL,
  closure_reason TEXT,
  closure_target TEXT,
  closure_required_at TEXT,
  claimed_at TEXT,
  last_nudge_attempt TEXT,
  last_nudge_result TEXT,
  last_heartbeat TEXT,
  resolution TEXT,
  target_repo TEXT,
  summary TEXT,
  evidence_ref TEXT,
  minting_generation_uuid TEXT,
  claimed_by_generation_uuid TEXT,
  human_intent TEXT,
  human_detail TEXT,
  reply_to TEXT,
  human_questions TEXT,
  human_answers TEXT,
  last_nudge_input_box TEXT,
  delegated_decision TEXT
);
CREATE TABLE snapshots (
  id TEXT PRIMARY KEY,
  rig_id TEXT NOT NULL,
  kind TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'complete',
  data TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE rigs (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  workspace_json TEXT,
  archived_at TEXT,
  permission_policy TEXT,
  rig_policy_origin TEXT,
  rig_policy_resolved_target TEXT,
  rig_policy_declaring_dir TEXT,
  rig_policy_launch_posture TEXT,
  claude_managed_block_file TEXT,
  non_interruptive INTEGER NOT NULL DEFAULT 0,
  install_root TEXT,
  env_json TEXT,
  startup_layers_json TEXT
);
CREATE TABLE schema_migrations (
  name TEXT PRIMARY KEY,
  applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT INTO schema_migrations (name) VALUES ('141_session_agent_runner.sql');
INSERT INTO events (type, payload, created_at) VALUES ('agent.session_identity', '{"sessionId":"s","runtime":"pi","sessionName":"seat"}', '2026-11-02 04:36:36');
