# 3. Data Model & Storage

> *"Data outlives code."* — Martin Kleppmann, Designing Data-Intensive Applications
>
> The database schema is the most stable part of any application. Code gets refactored, UIs get redesigned, but the shape of the data endures.

## Schema Design Principles

Life Manager's schema follows three principles:

1. **Every table has `user_id`** — even though all users currently map to `"default"`, the schema supports multi-tenancy from day one
2. **IDs are UUIDs** — generated server-side, no auto-increment sequences to conflict
3. **Timestamps are floats** — milliseconds since epoch, stored as `REAL` for sub-second precision

### The Six Tables

```sql
-- Core task tracking (shared by Todos and Groceries)
checklist_items (id, user_id, text, date, done, category, created_at, completed_by)

-- Package pickup tracking
shopee_packages (id, user_id, title, store, code, picked_up, created_at, completed_by)

-- Media watchlist
watch_items (id, user_id, text, media_type, done, created_at, completed_by)

-- Menstrual cycle history
cycles (id, user_id, start_date, end_date, symptoms)

-- Dynamic quick-add chips
default_items (id, user_id, category, text, created_at)

-- Schema versioning
migrations (name)
```

### The Checklist Unification

Todos and Groceries could have been separate tables. Instead, they share `checklist_items` with a `category` column discriminator (`"Todo"` or `"Grocery"`). This decision has three consequences:

1. **One API module** handles both — `api/checklist.rs` accepts `ItemCategory` as a parameter
2. **One component** renders both — `ChecklistPage` is parameterized by category
3. **One index** covers both — `idx_checklist_user ON (user_id, category, done)` serves both pages efficiently

The `ItemCategory` enum implements `Display` and `FromStr` for clean serialization:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ItemCategory { Todo, Grocery }

impl Display for ItemCategory {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ItemCategory::Todo => write!(f, "Todo"),
            ItemCategory::Grocery => write!(f, "Grocery"),
        }
    }
}
```

The `Copy` derive is critical — it allows `ItemCategory` to be captured in multiple closures without move conflicts.

## Connection Pooling

SQLite handles concurrency through file-level locking. While only one writer can operate at a time, multiple readers can work in parallel. The `r2d2` connection pool manages this:

```rust
static POOL: OnceLock<DbPool> = OnceLock::new();

pub fn init() {
    let manager = SqliteConnectionManager::file(db_path);
    let pool = Pool::new(manager).expect("Failed to create DB pool");
    // ... schema creation ...
    POOL.set(pool).expect("DB pool already initialized");
}

pub fn pool() -> &'static DbPool {
    POOL.get().expect("DB pool not initialized")
}
```

`OnceLock` guarantees the pool is initialized exactly once. The `'static` lifetime means it lives for the entire program — no reference counting, no Arc, no lifetime annotations in API functions.

## Migration Strategy

Life Manager uses a pragmatic migration approach: `CREATE TABLE IF NOT EXISTS` for schema creation and a `migrations` table for one-time data migrations.

```rust
fn run_once(conn: &rusqlite::Connection, name: &str, sql: &str) {
    let already_run: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM migrations WHERE name = ?1",
            params![name], |row| row.get(0),
        )
        .unwrap_or(false);

    if !already_run {
        let _ = conn.execute_batch(sql);
        let _ = conn.execute(
            "INSERT INTO migrations (name) VALUES (?1)",
            params![name],
        );
    }
}
```

For adding columns to existing tables, we use `ALTER TABLE ADD COLUMN` wrapped in an error-ignoring block — SQLite will error if the column already exists, and we silently ignore that:

```rust
let _ = conn.execute_batch(
    "ALTER TABLE checklist_items ADD COLUMN completed_by TEXT;"
);
```

This is crude but effective for a single-user app. For production systems with multiple instances, you'd want numbered migrations with a proper framework.

## Query Patterns

All queries use parameterized statements via `rusqlite::params![]`. This prevents SQL injection by construction — there is no string interpolation in any SQL query in the codebase.

```rust
// SAFE: parameterized query
conn.execute(
    "INSERT INTO checklist_items (id, user_id, text, date, done, category, created_at)
     VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
    params![id, user_id, text, date, cat_str, now],
)?;

// NEVER: string interpolation
// conn.execute(&format!("INSERT ... VALUES ('{text}')"), [])?;  // SQL INJECTION!
```

### Sorting Convention

All list queries follow the same sort order: **active items first, then completed, newest first within each group**:

```sql
ORDER BY done ASC, created_at DESC
```

This ensures the user always sees their actionable items at the top.

## The Symptoms JSON Column

The `cycles` table stores symptoms as a JSON array in a TEXT column:

```sql
symptoms TEXT NOT NULL DEFAULT '[]'
```

This is a deliberate denormalization. A normalized design would have a `cycle_symptoms` junction table with one row per symptom per cycle. But for a list of 0–7 short strings, JSON-in-a-column is simpler and faster — no joins needed, and the entire symptom list travels as a single value through the API.

Serialization uses serde:

```rust
// Write
let symptoms_json = serde_json::to_string(&symptoms)?;

// Read
let symptoms: Vec<String> = serde_json::from_str(&symptoms_json)
    .unwrap_or_default();  // Graceful degradation on corrupt data
```

## Input Validation

All write operations validate inputs server-side via `src/server/validate.rs`:

```rust
const MAX_TEXT: usize = 500;   // Item descriptions, titles
const MAX_SHORT: usize = 100;  // Codes, store names, symptoms

pub fn text(s: &str, field: &str) -> Result<(), ServerFnError> { ... }
pub fn short(s: &str, field: &str) -> Result<(), ServerFnError> { ... }
pub fn date(s: &str) -> Result<(), ServerFnError> { ... }
```

Validation happens after authentication but before any database access. This is the "validate at the boundary" principle — trust internal code, verify external input.
