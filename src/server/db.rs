use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use std::sync::OnceLock;

pub type DbPool = Pool<SqliteConnectionManager>;

static POOL: OnceLock<DbPool> = OnceLock::new();

pub fn init() {
    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "bookclub.db".to_string());
    let manager = SqliteConnectionManager::file(&db_path).with_init(|conn| {
        conn.pragma_update(None, "busy_timeout", 5000)?;
        // Enable cascade deletes on every pooled connection.
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(())
    });
    let pool = Pool::new(manager).expect("Failed to create DB pool");

    let conn = pool.get().expect("Failed to get DB connection");

    // WAL mode: crash-safe, better concurrent read/write performance
    conn.pragma_update(None, "journal_mode", "WAL")
        .expect("Failed to set WAL mode");
    // NORMAL sync is safe with WAL (full durability except on OS crash + power loss)
    conn.pragma_update(None, "synchronous", "NORMAL")
        .expect("Failed to set synchronous mode");
    // Wait up to 5s for locks instead of failing immediately
    conn.pragma_update(None, "busy_timeout", 5000)
        .expect("Failed to set busy_timeout");

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS books (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            author TEXT,
            cover_url TEXT,
            total_pages INTEGER,
            total_chapters INTEGER,
            description TEXT,
            google_books_id TEXT,
            isbn TEXT,
            added_by TEXT,
            created_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_books_created ON books(created_at);
        CREATE TABLE IF NOT EXISTS reading_progress (
            id TEXT PRIMARY KEY,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            reader TEXT NOT NULL,
            current_page INTEGER,
            current_chapter INTEGER,
            status TEXT NOT NULL DEFAULT 'to_read',
            updated_at REAL NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_progress_book_reader
            ON reading_progress(book_id, reader);
        CREATE TABLE IF NOT EXISTS book_comments (
            id TEXT PRIMARY KEY,
            book_id TEXT NOT NULL REFERENCES books(id) ON DELETE CASCADE,
            author TEXT NOT NULL,
            body TEXT NOT NULL,
            page INTEGER,
            chapter INTEGER,
            created_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_comments_book ON book_comments(book_id, created_at);
        CREATE TABLE IF NOT EXISTS notifications (
            id TEXT PRIMARY KEY,
            actor TEXT NOT NULL,
            action TEXT NOT NULL,
            module TEXT NOT NULL,
            item_text TEXT NOT NULL,
            created_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_notif_created ON notifications(created_at);
        CREATE TABLE IF NOT EXISTS notification_reads (
            user_name TEXT PRIMARY KEY,
            last_read_at REAL NOT NULL DEFAULT 0,
            cleared_at REAL NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS notification_settings (
            user_name TEXT PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS push_subscriptions (
            id TEXT PRIMARY KEY,
            user_name TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            p256dh TEXT NOT NULL,
            auth TEXT NOT NULL,
            created_at REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_push_user ON push_subscriptions(user_name);
        CREATE TABLE IF NOT EXISTS reader_aliases (
            login TEXT PRIMARY KEY,
            alias TEXT NOT NULL,
            updated_at REAL NOT NULL
        );",
    )
    .expect("Failed to run migrations");

    // Idempotent column add (safe to run repeatedly — "duplicate column" is expected).
    if let Err(e) = conn.execute("ALTER TABLE books ADD COLUMN toc_json TEXT", []) {
        let m = e.to_string();
        if !m.contains("duplicate column") {
            eprintln!("WARNING: toc_json migration failed: {m}");
        }
    }

    POOL.set(pool).expect("DB pool already initialized");
}

pub fn pool() -> &'static DbPool {
    POOL.get().expect("DB pool not initialized")
}
