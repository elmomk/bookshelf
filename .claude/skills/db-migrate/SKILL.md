---
name: db-migrate
description: Add a new table or column to the SQLite database schema
allowed-tools: Read, Edit, Bash, Grep
---

Add a database migration to `src/server/db.rs`.

## Context
- All tables live in `src/server/db.rs` in the `init()` function's `execute_batch` call
- Every table must have `user_id TEXT NOT NULL` for multi-user scoping (though all users map to "default")
- Use `CREATE TABLE IF NOT EXISTS` for new tables (idempotent)
- For new columns on existing tables, use `ALTER TABLE ... ADD COLUMN` wrapped in a separate `let _ = conn.execute_batch(...)` call (ignores "duplicate column" errors)
- Schema pattern: `id TEXT PRIMARY KEY, user_id TEXT NOT NULL, ...fields..., created_at REAL NOT NULL`

## Steps
1. Read `src/server/db.rs` to see existing schema
2. Add the migration (new table or ALTER TABLE)
3. If this is for an existing module, also update the corresponding model in `src/models/` and API in `src/api/`
4. Run `./scripts/check.sh` to verify compilation

Pass the table name and fields as arguments: `/db-migrate tablename field1:type field2:type`
