//! Construct authentic pre-copy schema fixtures without discarding old evidence.
pub fn remove_copy_protocol(db: &rusqlite::Connection) {
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM recovery_effects WHERE ordinal<>0",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "legacy fixtures cannot represent a new multi-effect inverse"
    );
    db.execute_batch("DROP TABLE recovery_dependencies; DROP TABLE recovery_groups; DROP TABLE copy_protocol;
        ALTER TABLE recovery_effects RENAME TO recovery_effects_new;
        CREATE TABLE recovery_effects(operation_id TEXT NOT NULL, original_step TEXT NOT NULL, inverse_step TEXT NOT NULL, parents TEXT NOT NULL, PRIMARY KEY(operation_id,original_step), UNIQUE(operation_id,inverse_step), FOREIGN KEY(operation_id,original_step) REFERENCES steps(operation_id,id), FOREIGN KEY(operation_id,inverse_step) REFERENCES steps(operation_id,id));
        INSERT INTO recovery_effects SELECT operation_id,original_step,inverse_step,parents FROM recovery_effects_new;
        DROP TABLE recovery_effects_new;").unwrap();
}
