# Static provider finding, separate from runtime qualification

The unchanged Vello supplier is
`2eae6243881d5b24c984d5ee35dde509ca5db782`.
In `research/vello_shaders/shader/clip_leaf.wgsl:30,80-103`, the prefix scan has
256 local invocations. Each invocation loads `reduced[local_id.x]` when
`local_id.x < wg_id.x`; the subsequent scan consumes only those 256 entries.
For workgroups with more preceding reductions, that code has no additional
scan level that loads entries beyond index 255.
`research/vello_encoding/src/config.rs:256-257` dispatches the reduction and
leaf groups from the encoded clip count without introducing another scan level.

This is a static source limitation. It does not establish a measured numeric
failure boundary, prove the cause of a particular damaged frame, or qualify
general Classic rendering with large clip streams. Counts inferred from retained
Scene layer pairs are not the same evidence as `master.encoding().n_clips`.
The replay's `netrender::tile_scopes` log records the actual encoded count and
cached tile count for each cold and warm frame.

The bounded Netrender change removes balanced, empty, normal source-over groups
after spatial draw filtering. It retains nonempty clip/alpha groups and empty
masks, blend/effect/backdrop groups. It neither changes Vello nor adds a hardcoded
provider cap. Any surviving corruption or large retained clip stream is a
separate provider tail requiring its own qualification.
