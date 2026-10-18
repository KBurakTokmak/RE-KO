CREATE TABLE IF NOT EXISTS world_boss_pools (
    pool_id TEXT PRIMARY KEY, zone INTEGER NOT NULL, name TEXT NOT NULL,
    respawn_ms INTEGER NOT NULL CHECK(respawn_ms>0)
);
CREATE TABLE IF NOT EXISTS world_boss_members (
    entity_id INTEGER PRIMARY KEY REFERENCES world_spawns(id),
    pool_id TEXT NOT NULL REFERENCES world_boss_pools(pool_id) ON DELETE CASCADE
);
