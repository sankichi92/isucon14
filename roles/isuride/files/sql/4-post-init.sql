UPDATE chair_locations cl
JOIN (
    SELECT
        chair_id,
        SUM(IFNULL(distance, 0)) AS total_distance
    FROM (
        SELECT
            chair_id,
            ABS(latitude - LAG(latitude) OVER (PARTITION BY chair_id ORDER BY created_at)) +
            ABS(longitude - LAG(longitude) OVER (PARTITION BY chair_id ORDER BY created_at)) AS distance
        FROM chair_locations
    ) tmp
    GROUP BY chair_id
) distance_table
ON cl.chair_id = distance_table.chair_id
SET cl.total_distance = distance_table.total_distance;
