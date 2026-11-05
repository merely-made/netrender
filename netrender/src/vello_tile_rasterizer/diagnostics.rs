// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0
use super::*;

impl VelloTileRasterizer {
    pub(crate) fn enable_native_diagnostics(&mut self, gpu_enabled: bool) {
        self.vello_renderer
            .set_diagnostic_cpu_clock(Some(crate::profiling::current_thread_cpu_time));
        self.vello_renderer
            .set_diagnostic_gpu(Some(std::sync::Arc::new(std::sync::Mutex::new(
                vello::diagnostics::GpuState::new(gpu_enabled),
            ))));
    }
    pub(crate) fn begin_diagnostic_batch(&mut self, id: u64) {
        self.vello_renderer.begin_diagnostic_batch(id);
    }
    pub(crate) fn poll_curve_quality_observed(
        &mut self,
    ) -> (
        Result<(usize, u64), vello::Error>,
        vello::diagnostics::QualityObservation,
    ) {
        let (result, observation) = self
            .vello_renderer
            .poll_curve_quality_observed(&self.handles.device);
        (
            result.map(|_| self.vello_renderer.pending_curve_quality()),
            observation,
        )
    }
    pub(crate) fn poll_diagnostic_gpu(&mut self) {
        self.vello_renderer.poll_diagnostic_gpu();
    }
    pub(crate) fn diagnostic_gpu_snapshot(&self, id: u64) -> vello::diagnostics::GpuReport {
        self.vello_renderer.diagnostic_gpu_snapshot(id)
    }
}
