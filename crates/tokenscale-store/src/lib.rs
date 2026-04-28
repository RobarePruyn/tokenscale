//! `tokenscale-store` — SQLite schema, migrations, and queries.
//!
//! All SQL lives in this crate. Other crates speak in terms of the domain types
//! defined in `tokenscale-core` and call typed query functions exposed here.
//!
//! The migrations directory is `migrations/` at the workspace root (not inside
//! this crate) so that operators can inspect the schema without spelunking into
//! a Cargo target tree. `sqlx::migrate!` references it via a relative path.
//!
//! Compile-time verification is on. The `.sqlx/` query cache at the workspace
//! root is committed; CI runs with `SQLX_OFFLINE=true`. After editing any
//! `sqlx::query!` invocation, run `cargo sqlx prepare --workspace`.

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
