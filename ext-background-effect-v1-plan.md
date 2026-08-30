# `ext-background-effect-v1` implementation plan

Issue: [#898](https://github.com/mahkoh/jay/issues/898)

Related research: [`ext-background-effect-v1-compositor-report.md`](ext-background-effect-v1-compositor-report.md)

## Summary

The protocol implementation is small, but applying blur correctly requires a new ordered graphics operation and changes to damage propagation. A blur operation must sample the scene rendered beneath a surface at a precise point in the bottom-to-top render stream. It also causes damage beneath the surface to affect a larger area, even when the blurred surface itself has not committed a new buffer.

The production implementation can be Vulkan-only. The protocol and software test renderer should be implemented first, followed by Vulkan. OpenGL should advertise no blur capability and should never receive blur operations. Scenes without blur should continue to use the existing optimized rendering paths.

Restricting the first implementation to Vulkan avoids adding effect shaders, framebuffer-copy support, and temporary-resource management to the legacy OpenGL renderer. OpenGL support can be added later without changing the protocol or surface-state model.

The initial implementation should use a fixed compositor blur policy. The protocol deliberately leaves the algorithm and strength to the compositor, and adding configuration would unnecessarily expand the first implementation across the configuration APIs.

## Protocol and surface state

Add wire definitions for `ext_background_effect_manager_v1` and `ext_background_effect_surface_v1`. Register the version 1 manager as a singleton global and send a `capabilities` event whenever a client binds it.

The manager creates at most one background-effect object for each `wl_surface`. Implement the specified `background_effect_exists` protocol error for duplicate objects. An effect object remains valid but inert after its `wl_surface` is destroyed; `set_blur_region` must then raise `surface_destroyed`.

Store both pending and committed blur regions on `WlSurface`:

- `set_blur_region` copies the current contents of the supplied `wl_region`.
- A null region removes the pending effect.
- The initial region is empty.
- Region changes take effect on the next `wl_surface.commit`.
- Destroying the effect object queues removal on the next commit.
- Committing a changed region damages the union of the old and new regions.

Destroying the effect object must release the surface's singleton slot immediately, even though removal of the committed effect remains pending until `wl_surface.commit`; this permits destroy-and-recreate before the commit. Add the pending field to `PendingState::merge()` so synchronized subsurface commits retain it. Clear references during client teardown to avoid retaining the surface/effect cycle. Add the manager and surface object types to the integration-test transport.

## Render operation

Add an ordered `GfxApiOp::Blur` operation. Keep two regions distinct in the operation:

- The paint region `B` is the exact effect region in framebuffer coordinates.
- The sample region `S` is `expand(B, r)` clipped to the framebuffer, where `r` is the integer physical support radius.
- The physical kernel contains `r` and the parameters needed to derive normalized weights.

Define blur strength in logical pixels so it has the same apparent size across output scales. Derive and cache the physical kernel for each render-pass scale, rounding its support radius outward. Carrying the kernel and both regions prevents each backend from deriving subtly different bounds or coefficients. The renderer should:

1. Render subsurfaces below the parent surface.
2. Emit the parent's blur operation.
3. Render the parent buffer.
4. Render subsurfaces above the parent surface.

This places the effect immediately behind the associated surface while preserving Wayland subsurface ordering. Convert region coordinates through Jay's client wire scale before clipping them to the main surface size and compositor bounds; KWin performs the equivalent client-to-compositor conversion. Then apply output scaling and transforms when constructing the framebuffer operation. Emit the operation only when the active render context reports blur support; committed protocol state remains stored while an unsupported context such as OpenGL is active.

A blur operation is a hard batch boundary. Backends must finish all preceding fills and texture copies before sampling the current render target. They may batch or sort contiguous operations between `Sync` and `Blur` boundaries, but must not move work across either boundary. The ordered path must also prevent opaque-region culling from removing lower scene content required inside `S`. Any render pass containing blur is ineligible for direct scanout.

## Damage propagation

Normal compositing is local: changing a pixel changes the same output pixel. Blur is non-local: an output pixel depends on source pixels within the filter radius.

Add a render-pass helper that derives two regions from the compositor's original damage:

- The presentation region contains all final output pixels that may have changed.
- The rendering region contains all pixels that must be recomputed to produce the presentation region.

Use conservative effect-granularity propagation initially. Jay's output damage is not tagged with scene depth, so treat all incoming damage as potentially below every blur. Walk operations from bottom to top; whenever the current damage intersects a blur's sample region `S`, add its complete paint region `B` and continue with the enlarged region so overlapping effects propagate causally. In reverse order, any required output inside `B` adds `S` to the region required from preceding operations. Region changes explicitly damage the complete old and new paint regions. If the region implementation cannot prove a smaller repaint across a chain of effects, falling back to the complete affected effect regions or output is correct.

A later optimization may use the tighter forward expression:

```text
B intersect expand(D, r)
```

That optimization is not required initially. Mature compositors generally repaint complete effect bounds because minimal partial blur damage is easy to get wrong.

Apply this before buffer-age accounting. Build the render pass before consuming connector damage, derive current presentation damage, and enqueue that expanded region in each buffer's `DamageQueue`. For the selected buffer, reverse-expand the queue's accumulated presentation region into a separate rendering region. `Latched` therefore needs both regions: pass the rendering region to `perform_render_pass()`, but only the accumulated presentation region to `copy_to_dev()`. Current presentation damage, rather than the larger source dependency, is what capture and KMS damage reporting should expose.

Region changes, surface movement, destruction, and capability changes must damage enough area to remove stale effects. Opaque visibility calculations must treat a blur as a dependency on lower content in `S`, not merely as a draw confined to `B`.

Add explicit handling for `GfxApiOp::Blur` to direct-scanout validation, lazy texture processing, the software renderer, and Vulkan. The OpenGL match should reject or log an unreachable unsupported operation rather than implement the effect. Exhaustive matches should make omitted paths visible during compilation.

## Blur algorithm

Use a truncated separable Gaussian with a finite, known support radius. A two-pass Gaussian is preferable for the initial implementation because its dependency radius is straightforward to model:

1. Compute the horizontal intermediate over `expand_y(B, r)`, sampling the current scene from `S`.
2. Sample that intermediate vertically while replacing pixels only in `B`.

Choose a fixed logical sigma and truncation factor, then derive the physical kernel and normalization while constructing the operation. Both backends consume those parameters. Initially use integer `texelFetch` taps in Vulkan: `BLEND_FEATURES` does not currently require `SAMPLED_IMAGE_FILTER_LINEAR`, so linear-sampling tap pairing would otherwise add an unprobed format requirement.

Sampling must extend outside `B`; only framebuffer edges use clamp-to-edge behavior. Because `texelFetch` does not apply sampler addressing modes, clamp its integer coordinates explicitly in the shader. Clamping at the effect-region edge would produce visible repeated-pixel borders. Blur should run in the compositor's active blend space; Vulkan's floating-point blend buffer is the preferred target where available. Preserve premultiplied-alpha conventions throughout the passes.

Cache temporary images by graphics context, format, and framebuffer size. Track their final layout, retain them in the pending frame, and reuse them only through the ordered graphics queue with the required barrier.

The final blur draw must replace the background in `B`, with blending disabled or equivalent replace factors, rather than alpha-blending another copy over it. Pixels outside `B` must retain the lower scene. The client surface is composited normally afterward.

## Backend scope

Do not implement blur in `GlRenderContext` initially. It should return `false` from `supports_background_blur()`, causing manager objects to advertise an empty capability set and preventing the renderer from emitting `GfxApiOp::Blur`. Switching from Vulkan to OpenGL must damage committed effect regions so stale blur is removed; switching back must restore the capability and effect without requiring a new client request.

The software graphics context may report support for deterministic integration testing, but Vulkan is the only production backend in scope.

## Vulkan backend

Vulkan is the largest part of the change. The current renderer partitions operations between blend-buffer and framebuffer passes and reorders operations within synchronization groups. That model cannot sample the partially composed scene at an arbitrary blur boundary.

Add an ordered-effect path selected only when a render pass contains blur. Keep the current path unchanged otherwise. In the ordered path, bypass blend-buffer elision and the normal blend/framebuffer partition: compose all operations that intersect the rendering region into the `ABGR16161616F` blend buffer in scene order. Existing sorting remains valid only within contiguous segments delimited by `Sync` and `Blur`.

The blend buffer already has `COLOR_ATTACHMENT` and `SAMPLED` usage, so no framebuffer copy or feedback-loop extension is required. The horizontal pass can sample the blend buffer into a separate intermediate image; only after that pass completes does the vertical pass write back to the blend buffer. Allocate that image through a dedicated cache or add an image-role key: the current `acquire_blend_buffer()` cache is keyed only by size and would return the scene blend image, creating the forbidden feedback loop. A blur pass must suppress both blend-buffer elision functions. If its caller did not allocate a blend buffer, the Vulkan renderer must acquire one for the target size or fail the frame rather than continue without the effect. Each blur operation should:

1. End the preceding dynamic-rendering instance after all lower operations have been recorded.
2. Barrier the blend buffer from color-attachment writes to fragment-shader sampling.
3. Barrier the intermediate image to color-attachment use and render the horizontal domain into it.
4. Barrier the intermediate image from color-attachment writes to shader sampling.
5. Barrier the blend buffer back to color-attachment use.
6. Resume dynamic rendering with `AttachmentLoadOp::LOAD`, not `DONT_CARE`, so the lower scene survives.
7. Render the vertical pass into exactly `B` with replace semantics.
8. Continue recording the associated surface and later operations until the next boundary.

The first blend-buffer rendering instance can use `DONT_CARE`, but the ordered path must clear the complete rendering region and then draw all intersecting scene operations; it must not reuse the normal opaque-clear subtraction. Every rendering instance resumed after a blur must use `LOAD`. This is a correctness requirement that the current single-pass `begin_rendering()` helper does not satisfy and therefore needs an explicit load-operation parameter or a dedicated ordered-path helper. With synchronization2, use color-attachment output/write to fragment-shader sampled-read dependencies before sampling, and the reverse dependency before writing an image again.

After all operations, use the existing color-managed output shader to copy presentation damage from the blend buffer to the scanout framebuffer. The ordered path must not render some opaque operations directly to the scanout framebuffer, because a later blur may need to sample them from the blend buffer.

Legacy-descriptor Vulkan is out of scope for the initial implementation. `acquire_blend_buffer()` already rejects those contexts, and implementing a second ordered offscreen and output path would largely duplicate the work. Advertise an empty capability set there. Supported Vulkan contexts are the non-legacy descriptor-buffer or descriptor-heap paths whose blend-format limits cover every active output framebuffer and whose blur pipelines initialize successfully.

Command buffers, image layouts, descriptor lifetimes, client acquire synchronization, and release synchronization must span the complete ordered effect sequence. Intermediate images must remain alive until the submitted frame completes. Multiple blur operations may reuse the same intermediate serially when the barriers and the graphics queue's submission order make that reuse safe.

## Dynamic capabilities

Expose a `GfxContext` blur-support query, defaulting to unsupported, together with the Vulkan blend-image limits. The manager's capability flags should be `BLUR` only for a non-legacy Vulkan context that has initialized the required descriptors and pipelines and whose blend-format limits cover every active output framebuffer size, and for the software test context. OpenGL and unsupported Vulkan contexts must send an empty capability set. Do not advertise based only on the graphics API name.

Track bound manager objects so `State::set_render_ctx` can resend capabilities after graphics API or device changes, and reevaluate the global capability when output sizes change. Since the protocol capability is global rather than per-output, one active target beyond the supported limits must disable it globally. If the capability disappears, rendering must immediately stop applying blur and damage all committed effect regions. Preserve committed client requests so they become active again if the capability returns, as required by the protocol.

A missing render context should produce an empty capability set rather than hiding the global. Once a blur operation has been emitted, a backend must either execute it or fail the frame; ordinary allocation or device failures do not change protocol capability by themselves.

## Tests

Cover the protocol state model: capability updates, singleton enforcement, copied and double-buffered regions, destruction, and inert objects. Extend the software graphics backend with deterministic blur and use one ordered scene with contrasting pixels outside `B` to verify masking, sampling from `S`, and subsurface placement.

Test damage-region derivation directly because the current software framebuffer ignores its `_region` argument and a screenshot alone cannot detect under-expansion. Include a chained pair of blur operations in that test. Validate the Vulkan path with synchronization validation enabled, and confirm that OpenGL and legacy-descriptor Vulkan advertise no blur.

## Implementation sequence

1. Add wire definitions, server objects, committed state, and protocol tests.
2. Add `GfxApiOp::Blur`, direct-scanout rejection, and damage-region derivation.
3. Implement deterministic software rendering and screenshot/damage tests.
4. Implement the Vulkan ordered-effect path and shader/resource management.
5. Add dynamic capability updates, feature documentation, and release notes.
6. Run the full checks, then formatting, and validate representative clients such as `foot`, `kitty`, or a small protocol test client.

The primary risks are Vulkan operation ordering, image transitions, and partial-damage correctness. The Wayland object model and double-buffered region state can follow existing surface extensions such as `wp_alpha_modifier_v1`, `wp_viewporter`, and `wp_fifo_v1`.
