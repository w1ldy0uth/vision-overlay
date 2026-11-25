SELECT MAX(arcana)
FROM (
    SELECT
        json_extract(snapshot_json, '$.current.arcana') AS arcana,
        json_extract(snapshot_json, '$.current.schedule_id') AS schedule_id,
        json_extract(snapshot_json, '$.current.starts_unix_s') AS starts,
        json_extract(snapshot_json, '$.current.ends_unix_s') AS ends
    FROM snapshots
    WHERE profile_id = ?1 AND collection = 'theater' AND scope = 'full'
    UNION ALL
    SELECT
        json_extract(value_json, '$.arcana'),
        json_extract(value_json, '$.schedule_id'),
        json_extract(value_json, '$.starts_unix_s'),
        json_extract(value_json, '$.ends_unix_s')
    FROM current_entities
    WHERE profile_id = ?1 AND collection = 'theater' AND entity_key = 'current'
)
WHERE schedule_id = ?2 AND starts = ?3 AND ends = ?4
  AND typeof(arcana) = 'integer' AND arcana BETWEEN 0 AND 2
