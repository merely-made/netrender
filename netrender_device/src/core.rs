/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! wgpu primitives owned by the renderer: instance, adapter, device,
//! queue. These come from the embedder via [`WgpuHandles`] (production)
//! or from [`boot`] (headless tests / CI).

/// wgpu features the renderer requires at boot. Phase 0.5 of the
/// netrender design plan demotes this to `Features::empty()` per
/// axiom 10 — Phases 1–9 work on a baseline wgpu adapter; optional
/// features like `DUAL_SOURCE_BLENDING` (Phase 10 subpixel-AA) move
/// to the pipeline factory that needs them. `IMMEDIATES` (wgpu 29's
/// rename of push constants) was previously requested unconditionally
/// for the smoke pipeline, but `brush_solid` declares
/// `immediate_size: 0`, so it's pure carry-over and dropped here.
pub const REQUIRED_FEATURES: wgpu::Features = wgpu::Features::empty();

/// The one limit netrender raises above wgpu's defaults. Stated once,
/// here, because every boot path has to apply it and a host that boots
/// its own device was previously copying the number.
pub const REQUIRED_INTER_STAGE_VARIABLES: u32 = 28;

/// Netrender's limits, raised over whatever a tenant asked for.
///
/// Takes the larger of each side rather than netrender's flat, so a
/// tenant that needs bigger buffers or more bind groups keeps them.
fn raised_for_netrender(mut limits: wgpu::Limits) -> wgpu::Limits {
    limits.max_inter_stage_shader_variables = limits
        .max_inter_stage_shader_variables
        .max(REQUIRED_INTER_STAGE_VARIABLES);
    limits
}

/// What another renderer needs from the device it will share with
/// netrender.
///
/// # The tenancy contract
///
/// A *tenant* is a second renderer drawing into the same frame as
/// netrender: a 3D scene under the page, a game world under its chrome.
/// The contract is deliberately narrow, and this type is the half of it
/// that has to be settled at boot, before either renderer exists:
///
/// 1. **One device, one queue.** Both renderers use the same
///    [`WgpuHandles`]. That is what lets the tenant's output be sampled
///    without a copy, and it is the whole reason this type exists — a
///    tenant that boots its own device gets a texture netrender cannot
///    read.
/// 2. **The tenant owns its target.** It renders into a texture it
///    created, and hands netrender a view. Netrender never learns what
///    a tenant draws.
/// 3. **Composition is explicit.** The host composites that view at a
///    stated scene-op boundary (`ExternalTextureComposite`), so whether
///    chrome lands over or under the tenant is a decision in the host's
///    code rather than an accident of draw order.
/// 4. **Receipts name the tenant.** A frame produced with a tenant
///    should say so where it reports timings, for the same reason the
///    rasterizer is named: a composed frame and a plain one are
///    different measurements.
///
/// Features are split by whether the tenant can do without them.
/// [`Self::required_features`] fail the boot when the adapter lacks
/// them; [`Self::optional_features`] are requested when present and
/// silently skipped when not, which is how a tenant asks for
/// `MULTI_DRAW_INDIRECT_COUNT` on a desktop adapter without ruling out
/// a thin one.
#[derive(Clone, Debug, Default)]
pub struct TenantNeeds {
    /// Features the tenant cannot work without.
    pub required_features: wgpu::Features,
    /// Features the tenant uses when the adapter has them.
    pub optional_features: wgpu::Features,
    /// Limits the tenant needs. Netrender's own minimums are raised
    /// over these, never under. `None` means wgpu's defaults.
    pub limits: Option<wgpu::Limits>,
    /// Device label, for captures and debug output.
    pub label: Option<&'static str>,
    /// Take every non-experimental feature the adapter offers, and its
    /// full limits. The shape of a JIT compute runtime (CubeCL boots
    /// its own devices exactly this way): the compiler probes adapter
    /// capability and emits against it, so a device holding less than
    /// the adapter fails shader validation at launch rather than boot.
    /// `MAPPABLE_PRIMARY_BUFFERS` stays excluded (a known performance
    /// trap CubeCL also excludes), and experimental features stay
    /// masked as everywhere else: a tenant wanting one names it in
    /// [`Self::required_features`].
    pub greedy: bool,
}

impl TenantNeeds {
    /// The features to ask the adapter for: everything required, plus
    /// whatever optional ones it actually has.
    ///
    /// Experimental features are dropped from the opportunistic half
    /// even when the adapter advertises them. wgpu 29 reports them as
    /// available but refuses the device unless they were asked for
    /// deliberately (`ExperimentalFeaturesNotEnabled`), so granting one
    /// because it happened to be there turns a working boot into a hard
    /// failure. A tenant that genuinely wants one names it in
    /// [`Self::required_features`], which is the deliberate ask wgpu is
    /// looking for.
    fn features(&self, available: wgpu::Features) -> wgpu::Features {
        let opportunistic = if self.greedy {
            (available - wgpu::Features::MAPPABLE_PRIMARY_BUFFERS)
                & !wgpu::Features::all_experimental_mask()
        } else {
            self.optional_features & available & !wgpu::Features::all_experimental_mask()
        };
        REQUIRED_FEATURES | self.required_features | opportunistic
    }

    fn limits(&self, adapter: &wgpu::Adapter) -> wgpu::Limits {
        if self.greedy {
            raised_for_netrender(adapter.limits())
        } else {
            raised_for_netrender(self.limits.clone().unwrap_or_default())
        }
    }
}

/// Bundle of wgpu primitives owned by the embedder and passed through
/// `create_netrender_instance` to the renderer. All four wgpu 29 handle
/// types are `Clone` (Arc-wrapped internally), so passing by value is
/// cheap.
///
/// The embedder is expected to have already created instance, adapter,
/// device, and queue for its own surface / compositor work; these
/// handles are *the same ones* the embedder uses, so external textures
/// (e.g. video frames) integrate naturally — they're created on the
/// same device and can be sampled here without copy.
#[derive(Clone)]
pub struct WgpuHandles {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[derive(Debug)]
pub enum BootError {
    Adapter(wgpu::RequestAdapterError),
    MissingFeatures(wgpu::Features),
    Device(wgpu::RequestDeviceError),
}

impl std::fmt::Display for BootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Adapter(e) => write!(f, "could not request a wgpu adapter: {e}"),
            Self::MissingFeatures(missing) => {
                write!(f, "adapter is missing required features: {missing:?}")
            }
            Self::Device(e) => write!(f, "device request failed: {e}"),
        }
    }
}

impl std::error::Error for BootError {}

impl From<wgpu::RequestAdapterError> for BootError {
    fn from(e: wgpu::RequestAdapterError) -> Self {
        Self::Adapter(e)
    }
}

impl From<wgpu::RequestDeviceError> for BootError {
    fn from(e: wgpu::RequestDeviceError) -> Self {
        Self::Device(e)
    }
}

/// Async boot core: create the instance, pick an adapter, verify
/// [`REQUIRED_FEATURES`], request a device + queue. This is the
/// portable shape — browser / wasm32-unknown-unknown consumers must
/// drive it from their own runtime (`wasm-bindgen-futures`, etc.)
/// because `wgpu`'s adapter / device requests are inherently async on
/// the web.
///
/// Phase 0.5 demoted [`REQUIRED_FEATURES`] to `Features::empty()`, so
/// this boots cleanly on Lavapipe / WARP / SwiftShader software
/// adapters.
pub async fn boot_async() -> Result<WgpuHandles, BootError> {
    boot_async_with(default_backends()).await
}

/// The backend set, honoring `WGPU_BACKEND` (`dx12` / `vulkan` / `gl` / …) when
/// set, else all available. `wgpu::Instance::default()` ignores the env, which is
/// why a bare boot lands on the adapter wgpu prefers (Vulkan on Windows+NVIDIA);
/// this restores the env override.
fn default_backends() -> wgpu::Backends {
    wgpu::Backends::from_env().unwrap_or_else(wgpu::Backends::all)
}

/// Boot with an explicit backend set (e.g. a host that wants D3D12 for same-API
/// system-WebView texture import). `WgpuHandles` is otherwise identical.
pub async fn boot_async_with(backends: wgpu::Backends) -> Result<WgpuHandles, BootError> {
    boot_async_shared(backends, None, &TenantNeeds::default()).await
}

/// Boot one device for netrender **and a tenant renderer**.
///
/// The tenancy contract is on [`TenantNeeds`]; this is the boot half of
/// it. A host that draws a 3D scene under its chrome states what that
/// renderer needs and gets handles both can use, instead of running the
/// adapter dance itself and hoping the two feature sets agree. Booting
/// separately is the failure this prevents: two devices cannot share a
/// texture, so the composite silently has nothing to sample.
///
/// `compatible` is the surface the handles must be able to present to,
/// when the host has already created one.
///
/// Fails with [`BootError::MissingFeatures`] naming exactly what is
/// absent, whether the gap is netrender's or the tenant's, rather than
/// failing later at pipeline creation where the cause is unrecoverable.
pub async fn boot_async_shared(
    backends: wgpu::Backends,
    compatible: Option<&wgpu::Surface<'_>>,
    needs: &TenantNeeds,
) -> Result<WgpuHandles, BootError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends,
        flags: wgpu::InstanceFlags::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::default(),
        display: None,
    });
    boot_async_on(instance, compatible, needs).await
}

/// [`boot_async_shared`] against an instance the host already made.
///
/// Separate because a windowing host creates its instance before it has
/// a surface to be compatible with, and the instance must be the same
/// one the surface came from.
pub async fn boot_async_on(
    instance: wgpu::Instance,
    compatible: Option<&wgpu::Surface<'_>>,
    needs: &TenantNeeds,
) -> Result<WgpuHandles, BootError> {
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: compatible,
            force_fallback_adapter: false,
        })
        .await?;

    // Required only. Optional tenant features are dropped rather than
    // demanded, which is the difference between a thin adapter running
    // without them and refusing to boot at all.
    let missing = (REQUIRED_FEATURES | needs.required_features) - adapter.features();
    if !missing.is_empty() {
        return Err(BootError::MissingFeatures(missing));
    }

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some(needs.label.unwrap_or("netrender device")),
            required_features: needs.features(adapter.features()),
            required_limits: needs.limits(&adapter),
            ..Default::default()
        })
        .await?;

    Ok(WgpuHandles {
        instance,
        adapter,
        device,
        queue,
    })
}

/// Blocking boot helper for desktop tests, CI goldens, and tools that
/// don't have an embedder fixture. Production goes through
/// [`crate::WgpuDevice::with_external`] where the embedder supplies
/// the primitives.
///
/// Not available on `wasm32-unknown-unknown`: `pollster::block_on`
/// panics there because the browser provides no executor to drive the
/// adapter / device futures. Browser / WASM consumers should call
/// [`boot_async`] directly from `wasm-bindgen-futures::spawn_local`
/// (or equivalent).
#[cfg(not(target_arch = "wasm32"))]
pub fn boot() -> Result<WgpuHandles, BootError> {
    pollster::block_on(boot_async())
}

/// Blocking [`boot_async_with`] — boot with an explicit backend set.
#[cfg(not(target_arch = "wasm32"))]
pub fn boot_with(backends: wgpu::Backends) -> Result<WgpuHandles, BootError> {
    pollster::block_on(boot_async_with(backends))
}

/// Blocking [`boot_async_shared`] — one device for netrender and a
/// tenant renderer.
#[cfg(not(target_arch = "wasm32"))]
pub fn boot_shared(
    backends: wgpu::Backends,
    compatible: Option<&wgpu::Surface<'_>>,
    needs: &TenantNeeds,
) -> Result<WgpuHandles, BootError> {
    pollster::block_on(boot_async_shared(backends, compatible, needs))
}

/// Blocking [`boot_async_on`] — share the host's own instance.
#[cfg(not(target_arch = "wasm32"))]
pub fn boot_on(
    instance: wgpu::Instance,
    compatible: Option<&wgpu::Surface<'_>>,
    needs: &TenantNeeds,
) -> Result<WgpuHandles, BootError> {
    pollster::block_on(boot_async_on(instance, compatible, needs))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Boot the device, clear a 4×4 offscreen target to a known color,
    /// read it back, assert the pixel matches. Smallest end-to-end
    /// receipt for the device path.
    #[test]
    fn boot_clear_readback_smoke() {
        let dev = boot().expect("wgpu boot");

        let size = wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        };
        let texture = dev.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("S1 smoke target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = dev
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("S1 smoke encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("S1 smoke pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        let padded_bytes_per_row = (4 * 4_u32).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = dev.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("S1 smoke readback"),
            size: padded_bytes_per_row as u64 * 4,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y: 0, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(4),
                },
            },
            size,
        );
        dev.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        dev.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll");
        rx.recv()
            .expect("map_async sender dropped")
            .expect("map failed");

        let mapped = slice.get_mapped_range();
        // Rgba8Unorm: clear (1.0, 0.0, 0.0, 1.0) → (255, 0, 0, 255).
        assert_eq!(&mapped[0..4], &[255, 0, 0, 255]);
    }
}
