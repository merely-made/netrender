// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
use super::*;
use crate::profiling::ThreadCpuStamp;
use std::time::Instant;

pub use vello::diagnostics::{GpuReport, GpuStatus, QualityObservation, WorkSpan};

#[derive(Clone, Copy, Debug, Default)]
pub struct ClassicQualityObservation {
    pub lock: WorkSpan,
    pub provider: QualityObservation,
    pub gpu_readback: WorkSpan,
}

fn measured(wall: Instant, cpu: ThreadCpuStamp) -> WorkSpan {
    let thread_cpu = ThreadCpuStamp::now().elapsed_since(cpu);
    WorkSpan {
        wall: wall.elapsed(),
        thread_cpu,
        calls: 1,
    }
}

impl Renderer {
    pub fn enable_native_diagnostics(&self, gpu_enabled: bool) -> bool {
        let Some(rast) = &self.vello_rasterizer else {
            return false;
        };
        rast.lock()
            .expect("vello_rasterizer lock")
            .enable_native_diagnostics(gpu_enabled);
        true
    }
    pub fn begin_diagnostic_batch(&self, id: u64) {
        if let Some(rast) = &self.vello_rasterizer {
            rast.lock()
                .expect("vello_rasterizer lock")
                .begin_diagnostic_batch(id);
        }
    }
    pub fn poll_vello_curve_quality_observed(
        &self,
    ) -> (
        Result<Option<(usize, u64)>, vello::Error>,
        ClassicQualityObservation,
    ) {
        let Some(rast) = &self.vello_rasterizer else {
            return (Ok(None), ClassicQualityObservation::default());
        };
        let wall = Instant::now();
        let cpu = ThreadCpuStamp::now();
        let mut rast = rast.lock().expect("vello_rasterizer lock");
        let lock = measured(wall, cpu);
        let (result, provider) = rast.poll_curve_quality_observed();
        let wall = Instant::now();
        let cpu = ThreadCpuStamp::now();
        rast.poll_diagnostic_gpu();
        let gpu_readback = measured(wall, cpu);
        (
            result.map(Some),
            ClassicQualityObservation {
                lock,
                provider,
                gpu_readback,
            },
        )
    }
    /// Read-only metadata: no mapped-buffer access or unmap outside worker scope.
    pub fn diagnostic_gpu_snapshot(&self, id: u64) -> GpuReport {
        self.vello_rasterizer.as_ref().map_or_else(
            || GpuReport::disabled(id),
            |rast| {
                rast.lock()
                    .expect("vello_rasterizer lock")
                    .diagnostic_gpu_snapshot(id)
            },
        )
    }
}
