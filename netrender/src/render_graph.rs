// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! Phase 6 render-task graph.
//!
//! Tasks declare their input dependencies by ID; the graph topo-sorts
//! (Kahn's algorithm) and encodes all passes into a single
//! `CommandEncoder`. Each task's output texture is allocated at execute
//! time; callers supply pre-existing textures for "external" sources
//! (uploaded images, prior frame tiles, etc.).
//!
//! Typical use: build blur or filter sub-graphs, execute them before
//! `Renderer::prepare`, insert the resulting textures into the image
//! cache via `ImageCache::insert_gpu`, then composite them as images
//! in the main scene pass.

use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::fmt;

/// Stable identifier for a task's output texture within one graph execution.
pub type TaskId = u64;

/// Signature for a task's encode callback.
///
/// Receives the wgpu device (for bind group creation), the active encoder,
/// and a slice of input texture views in exactly the order listed in
/// [`Task::inputs`], followed by the pre-created output view. Should encode
/// exactly one render pass targeting `output`.
pub type EncodeCallback = Box<
    dyn FnOnce(&wgpu::Device, &mut wgpu::CommandEncoder, &[wgpu::TextureView], &wgpu::TextureView)
        + Send,
>;

/// One node in the render-task graph.
pub struct Task {
    pub id: TaskId,
    /// Pixel dimensions of the output texture.
    pub extent: wgpu::Extent3d,
    /// Format of the output texture.
    pub format: wgpu::TextureFormat,
    /// IDs whose output textures must be ready before this task runs.
    /// External IDs (supplied via `execute`'s `externals` map) are valid
    /// here and are treated as already-complete leaf nodes.
    pub inputs: Vec<TaskId>,
    /// Encode callback: builds and submits one render pass for this task.
    pub encode: EncodeCallback,
}

/// A malformed render-task graph that cannot be safely executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderGraphError {
    /// More than one task declared the same output ID.
    DuplicateTaskId { task_id: TaskId },
    /// An external texture ID conflicts with a registered task output ID.
    ExternalTaskIdCollision { task_id: TaskId },
    /// A task input is neither another task output nor an external texture.
    MissingInput { task_id: TaskId, input_id: TaskId },
    /// Registered task dependencies contain a cycle. `task_ids` contains the
    /// insertion-ordered residual tasks blocked by that cycle, not an exact SCC.
    Cycle { task_ids: Vec<TaskId> },
}

impl fmt::Display for RenderGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTaskId { task_id } => {
                write!(f, "render graph contains duplicate task ID {task_id}")
            }
            Self::ExternalTaskIdCollision { task_id } => {
                write!(
                    f,
                    "render graph external texture ID {task_id} conflicts with a task output"
                )
            }
            Self::MissingInput { task_id, input_id } => {
                write!(
                    f,
                    "render graph task {task_id} requires missing input {input_id}"
                )
            }
            Self::Cycle { task_ids } => {
                write!(
                    f,
                    "render graph dependency cycle leaves task IDs {task_ids:?} blocked"
                )
            }
        }
    }
}

impl Error for RenderGraphError {}

/// Directed acyclic graph of render tasks.
///
/// Build with [`RenderGraph::push`], execute with [`RenderGraph::execute`].
/// A single `CommandEncoder` is used for all passes; the GPU processes
/// them in the submitted order (which matches the topo-sorted dependency
/// order).
pub struct RenderGraph {
    tasks: Vec<Task>,
}

impl RenderGraph {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    /// Add a task to the graph. Tasks may be pushed in any order;
    /// `execute` will sort them by dependency before encoding.
    pub fn push(&mut self, task: Task) {
        self.tasks.push(task);
    }

    /// Topo-sort, allocate output textures, and encode all passes into
    /// one command submission.
    ///
    /// `externals` supplies pre-existing textures for IDs that are not
    /// registered as tasks (source images, tile inputs, etc.). They are
    /// treated as already-complete leaf nodes during the topo-sort.
    ///
    /// Returns a map of `TaskId → wgpu::Texture` containing both the
    /// externals and the newly-created task outputs. The caller can
    /// extract specific outputs by ID (e.g. to insert into the image
    /// cache for compositing in the next scene pass). Returns a typed error
    /// before encoding if task/external IDs conflict, an input is unavailable,
    /// or the task dependencies cycle.
    pub fn execute(
        self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        externals: HashMap<TaskId, wgpu::Texture>,
    ) -> Result<HashMap<TaskId, wgpu::Texture>, RenderGraphError> {
        let external_ids: HashSet<TaskId> = externals.keys().copied().collect();
        let sorted = topo_sort(&self.tasks, &external_ids)?;
        let mut tasks: Vec<Option<Task>> = self.tasks.into_iter().map(Some).collect();

        let mut outputs: HashMap<TaskId, wgpu::Texture> = externals;

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("render_graph"),
        });

        for index in sorted {
            let task = tasks[index]
                .take()
                .expect("task present after validated topo-sort");

            let output_tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("render_graph task output"),
                size: task.extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: task.format,
                // TEXTURE_BINDING so later tasks / the image cache can read the output.
                // COPY_SRC so test code can read back pixels via read_rgba8_texture.
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let output_view = output_tex.create_view(&wgpu::TextureViewDescriptor::default());

            // Validation guarantees each input is present at this point. Mapping
            // directly over `Task::inputs` preserves its callback contract even
            // when an input ID occurs more than once.
            let input_views: Vec<wgpu::TextureView> = task
                .inputs
                .iter()
                .map(|input_id| {
                    outputs
                        .get(input_id)
                        .expect("validated input available before callback")
                        .create_view(&wgpu::TextureViewDescriptor::default())
                })
                .collect();
            debug_assert_eq!(input_views.len(), task.inputs.len());

            (task.encode)(device, &mut encoder, &input_views, &output_view);

            outputs.insert(task.id, output_tex);
        }

        queue.submit(std::iter::once(encoder.finish()));
        Ok(outputs)
    }
}

/// Kahn's algorithm over the registered tasks.
///
/// External IDs are treated as already-satisfied (in-degree contribution is
/// zero from them). Both the initial ready queue and newly-ready dependents
/// retain `RenderGraph::push` insertion order.
fn topo_sort(
    tasks: &[Task],
    external_ids: &HashSet<TaskId>,
) -> Result<Vec<usize>, RenderGraphError> {
    let mut task_indices = HashMap::with_capacity(tasks.len());
    for (index, task) in tasks.iter().enumerate() {
        if task_indices.insert(task.id, index).is_some() {
            return Err(RenderGraphError::DuplicateTaskId { task_id: task.id });
        }
    }
    for task in tasks {
        if external_ids.contains(&task.id) {
            return Err(RenderGraphError::ExternalTaskIdCollision { task_id: task.id });
        }
    }

    // in_degree: how many registered-task inputs each task is still waiting on.
    let mut in_degree = vec![0usize; tasks.len()];
    // rev: for each registered task output, the insertion-ordered tasks that
    // depend on it.
    let mut rev = vec![Vec::new(); tasks.len()];

    for (task_index, task) in tasks.iter().enumerate() {
        for &input_id in &task.inputs {
            if let Some(&input_index) = task_indices.get(&input_id) {
                in_degree[task_index] += 1;
                rev[input_index].push(task_index);
            } else if !external_ids.contains(&input_id) {
                return Err(RenderGraphError::MissingInput {
                    task_id: task.id,
                    input_id,
                });
            }
        }
    }

    let mut queue: VecDeque<usize> = (0..tasks.len())
        .filter(|&index| in_degree[index] == 0)
        .collect();

    let mut result = Vec::with_capacity(tasks.len());
    while let Some(index) = queue.pop_front() {
        result.push(index);
        for &dependent_index in &rev[index] {
            let degree = &mut in_degree[dependent_index];
            *degree -= 1;
            if *degree == 0 {
                queue.push_back(dependent_index);
            }
        }
    }

    if result.len() != tasks.len() {
        let task_ids = tasks
            .iter()
            .enumerate()
            .filter_map(|(index, task)| (in_degree[index] > 0).then_some(task.id))
            .collect();
        return Err(RenderGraphError::Cycle { task_ids });
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: TaskId, inputs: Vec<TaskId>) -> Task {
        Task {
            id,
            extent: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            format: wgpu::TextureFormat::Rgba8Unorm,
            inputs,
            encode: Box::new(|_, _, _, _| {}),
        }
    }

    #[test]
    fn topo_sort_keeps_insertion_order_for_ready_tasks() {
        let tasks = vec![
            task(30, vec![10]),
            task(20, vec![]),
            task(10, vec![]),
            task(40, vec![10]),
        ];

        let order = topo_sort(&tasks, &HashSet::new()).expect("valid graph");
        let ids: Vec<_> = order.into_iter().map(|index| tasks[index].id).collect();

        assert_eq!(ids, vec![20, 10, 30, 40]);
    }

    #[test]
    fn topo_sort_rejects_duplicate_task_ids() {
        let error = topo_sort(&[task(7, vec![]), task(7, vec![])], &HashSet::new())
            .expect_err("duplicate ID must be refused");

        assert_eq!(error, RenderGraphError::DuplicateTaskId { task_id: 7 });
    }

    #[test]
    fn topo_sort_rejects_missing_inputs() {
        let error = topo_sort(&[task(4, vec![99])], &HashSet::new())
            .expect_err("missing input must be refused");

        assert_eq!(
            error,
            RenderGraphError::MissingInput {
                task_id: 4,
                input_id: 99,
            }
        );
    }

    #[test]
    fn topo_sort_rejects_external_task_id_collisions() {
        let error = topo_sort(&[task(7, vec![])], &HashSet::from([7]))
            .expect_err("external/task ID collision must be refused");

        assert_eq!(
            error,
            RenderGraphError::ExternalTaskIdCollision { task_id: 7 }
        );
    }

    #[test]
    fn topo_sort_preserves_repeated_input_multiplicity() {
        let tasks = vec![task(2, vec![1, 1]), task(1, vec![])];

        let order = topo_sort(&tasks, &HashSet::new()).expect("valid graph");
        let ids: Vec<_> = order.into_iter().map(|index| tasks[index].id).collect();

        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn topo_sort_rejects_cycles() {
        let error = topo_sort(&[task(8, vec![9]), task(9, vec![8])], &HashSet::new())
            .expect_err("cycle must be refused");

        assert_eq!(
            error,
            RenderGraphError::Cycle {
                task_ids: vec![8, 9]
            }
        );
    }
}
