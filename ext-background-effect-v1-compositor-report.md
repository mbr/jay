# `ext-background-effect-v1` compositor implementation report

This report examines established Wayland compositors that implement client-requested background blur, with emphasis on `ext-background-effect-v1` and lessons applicable to Jay's Vulkan renderer. "Wayland compositor" is the usual term for these projects.

## Scope

The following current source trees implement `ext-background-effect-v1` and render the requested blur:

| Compositor | Source revision examined | Rendering stack | Blur algorithm |
| --- | --- | --- | --- |
| KWin | [`23b04278`](https://invent.kde.org/plasma/kwin/-/commit/23b04278648a3a86d5b8ebd70a331587e274c89c) | OpenGL | Dual Kawase |
| niri | [`dd75865f`](https://github.com/niri-wm/niri/commit/dd75865f547f0eac0e9b6c4d86d2cd00c0744252) | Smithay GLES | Dual Kawase |
| COSMIC compositor | [`5c930945`](https://github.com/pop-os/cosmic-comp/commit/5c93094574b31800caaa27255caac4a7253f24ad) | Smithay GLES | Dual Kawase |
| Hyprland | [`60695e18`](https://github.com/hyprwm/Hyprland/commit/60695e18c8cd17399651a9af5db3f42d1867e1e6) | Custom OpenGL renderer | Dual Kawase |
| Mutter | [`e730d1e6`](https://gitlab.gnome.org/GNOME/mutter/-/commit/e730d1e6fc006bf2e83371fd22c92160122c57ff) | Clutter/Cogl | Downscaled separable Gaussian |

Smithay revision [`e3d461a0`](https://github.com/Smithay/smithay/commit/e3d461a057ba244d213a8498ec372b0799cca103), used by niri and COSMIC, provides both the protocol implementation and a generic ordered framebuffer-effect facility.

SwayFX/SceneFX and Wayfire also have established background blur implementations, but their current source trees do not implement `ext-background-effect-v1`. They are covered separately because their damage handling is still relevant.

The most important overall finding is that there is no established Vulkan implementation to copy. Every exact protocol implementation found uses OpenGL or GLES, either directly or through Cogl. Wayfire also documents that its blur plugin is unavailable on its experimental Vulkan backend. Jay will therefore need to translate established rendering patterns into Vulkan rather than port an existing Vulkan implementation.

## Common architecture

Despite different scene graphs, all five protocol implementations converge on the same rendering model:

1. Store the requested region as double-buffered `wl_surface` state.
2. Insert an effect immediately before the corresponding surface is drawn.
3. Snapshot or sample the framebuffer containing everything rendered below that surface.
4. Blur through one or more intermediate textures.
5. replace only the requested effect region with the blurred result.
6. Draw the client surface normally over it.

None treats client-requested blur as a final full-output post-process. Ordered execution is necessary for overlapping blurred surfaces: a later blur sees the already composited result of earlier surfaces.

Dual Kawase is the dominant algorithm. Mutter is the exception, using a separable Gaussian with dynamic downscaling. All implementations use cached or reusable intermediate textures and conservatively enlarge damage around blur dependencies.

## KWin

### Protocol state

KWin's protocol implementation is in [`src/wayland/backgroundeffect_v1.cpp`](https://invent.kde.org/plasma/kwin/-/blob/23b04278648a3a86d5b8ebd70a331587e274c89c/src/wayland/backgroundeffect_v1.cpp). It:

- Tracks one `ExtBackgroundEffectSurfaceV1` on `SurfaceInterfacePrivate`.
- Posts `background_effect_exists` for a duplicate object.
- Copies the `RegionInterface` region into pending surface state.
- Marks a dedicated `SurfaceState::Field::Blur` bit.
- Clears pending blur state when the effect object is destroyed.
- Makes the object inert after the underlying surface is destroyed.
- Posts `surface_destroyed` if an inert object receives `set_blur_region`.

The pending region is committed by the normal surface-state machinery in [`surface.cpp`](https://invent.kde.org/plasma/kwin/-/blob/23b04278648a3a86d5b8ebd70a331587e274c89c/src/wayland/surface.cpp), which emits `blurChanged` only after commit.

KWin has the strongest capability precedent for Jay. The protocol global always exists, but the blur effect plugin calls `addBlurCapability()` only after all shaders and resources initialize successfully. Its destructor calls `removeBlurCapability()`. The manager broadcasts changes to all bound resources and delays removal by 100 ms so reloading the effect does not cause clients to churn state. This directly supports Jay's plan to expose `BLUR` only while a suitable Vulkan context is active.

### Rendering

The renderer is implemented as KWin's built-in blur effect in [`src/plugins/blur/blur.cpp`](https://invent.kde.org/plasma/kwin/-/blob/23b04278648a3a86d5b8ebd70a331587e274c89c/src/plugins/blur/blur.cpp). `BlurEffect::drawWindow()` calls `blur()` and then delegates to the normal window draw, giving the required ordering.

For each window and render view, KWin caches a texture/framebuffer pyramid. Level zero stores the unblurred background behind the bounding rectangle; each subsequent level is half the previous size. At render time KWin:

1. Computes the transformed client blur region and clips it to current device damage.
2. Copies dirty source rectangles from the current render target into level zero.
3. Runs one or more Dual-Kawase downsample passes.
4. Runs upsample passes back toward full resolution.
5. Draws the blurred texture through geometry generated from the requested region.
6. Applies saturation and optional additive noise in the final pass.
7. Draws the window over the result.

The copied source and temporary texture use the render target's internal format. Render data is kept per `RenderView`, because views can differ in color space and visible windows.

KWin uses the bounding rectangle as the processing domain but uses the exact region rectangles for the final draw. It also supports compositor-provided decoration regions, internal windows, and the older X11 blur property through the same renderer.

### Damage and scanout

KWin adds a `BackgroundEffectItem` below the window in the scene graph. [`WorkspaceScene::accumulateRepaints()`](https://invent.kde.org/plasma/kwin/-/blob/23b04278648a3a86d5b8ebd70a331587e274c89c/src/scene/workspacescene.cpp) checks whether accumulated damage from lower items intersects that effect item. If it does, it damages the effect bounds and expands the region that must be rendered beneath otherwise opaque items by a configured blur support size.

The level-zero texture is persistent, so only dirty source rectangles need to be copied. Unchanged source pixels remain cached for subsequent partial repaints.

`BlurEffect::blocksDirectScanout()` returns `false`. This is intentional because a directly scanned-out opaque fullscreen window completely hides its requested background effect; meaningful blur normally already makes the scene ineligible for direct scanout. Jay's simpler rule of rejecting direct scanout whenever an actual blur operation is emitted remains safer for an initial implementation.

### Lessons for Jay

KWin is the best reference for dynamic capabilities, exact region masking, and per-effect source caching. Its per-window pyramid would produce many Vulkan images, however. Jay's existing reusable output-sized blend buffers make a shared ordered target more attractive than copying KWin's allocation model directly.

## niri

### Protocol state

niri delegates the wire protocol and core double buffering to Smithay. Its handler is in [`src/handlers/background_effect.rs`](https://github.com/niri-wm/niri/blob/dd75865f547f0eac0e9b6c4d86d2cd00c0744252/src/handlers/background_effect.rs).

Smithay stores a copied `RegionAttributes` in `BackgroundEffectSurfaceCachedState`. niri installs a post-commit hook when a request changes pending state. After commit it marks a per-surface processed-region cache dirty and damages the associated effect. Region add/subtract operations are lazily normalized into non-overlapping rectangles and stored behind an `Arc`.

niri always returns the `Blur` capability. Its global `blur { off }` setting suppresses rendering without changing the advertised capability. That policy is not suitable for Jay's Vulkan-only support, where an OpenGL context is technically unable to execute the operation.

### Rendering

The effect integration is in [`background_effect.rs`](https://github.com/niri-wm/niri/blob/dd75865f547f0eac0e9b6c4d86d2cd00c0744252/src/render_helpers/background_effect.rs), [`framebuffer_effect.rs`](https://github.com/niri-wm/niri/blob/dd75865f547f0eac0e9b6c4d86d2cd00c0744252/src/render_helpers/framebuffer_effect.rs), and [`blur.rs`](https://github.com/niri-wm/niri/blob/dd75865f547f0eac0e9b6c4d86d2cd00c0744252/src/render_helpers/blur.rs).

niri added a generic `is_framebuffer_effect()` render-element concept to its Smithay renderer. When the renderer reaches such an element in bottom-to-top order, it first calls `capture_framebuffer()`, then calls the normal `draw()`. The background effect element therefore becomes an explicit ordered barrier, very similar to the proposed `GfxApiOp::Blur` in Jay.

For normal, non-xray blur, niri:

1. Uses `glBlitFramebuffer` to copy the current framebuffer under the effect geometry into an `ABGR8888` texture.
2. Allocates or reuses a chain of half-sized textures.
3. Runs Dual-Kawase downsample and upsample shaders.
4. Draws the full-size result back through the requested subregion.
5. Applies optional saturation, noise, and rounded-corner clipping in the final draw.

The shaders use five taps for downsampling and eight weighted taps for upsampling. Texture sampling is `CLAMP_TO_EDGE`. The default configuration uses three passes and an offset of three.

niri also implements an `xray` optimization and enables it by default whenever a background effect is active. Xray blur samples cached wallpaper/workspace background buffers and deliberately ignores intervening windows. The cached background can be blurred once and reused by every window. A rule can select non-xray blur to capture the actual scene beneath a window, but niri documents that path as experimental.

This is a compositor policy choice, not a useful model for Jay's first implementation. Jay should render the actual ordered background expected from the protocol rather than silently substituting wallpaper-only blur.

### Damage and limitations

The shared Smithay damage tracker recognizes framebuffer-effect elements. If damage from elements below intersects an effect's geometry, Smithay marks the effect for recapture, damages its full geometry, and weakens opaque-region culling above it so all source pixels are available.

niri's non-xray capture bounds are the effect geometry itself, without an external support border. The shader clamps at those bounds. This avoids outside-region damage dependencies but causes the blur near an effect edge to repeat edge pixels rather than sample neighboring scene pixels. COSMIC's extended capture bounds are a better reference for Jay.

The framebuffer-effect API automatically prevents direct scanout in Smithay's DRM compositor. niri keeps separate caches per render target, allowing output, screenshot, and screencast paths to use the same effect machinery without sharing stale textures.

### Lessons for Jay

niri most clearly validates the proposed ordered backend-neutral render operation. Its separation between framebuffer capture, filter execution, and exact-region drawing maps cleanly to Vulkan commands. Its xray default and unpadded non-xray source should not be copied.

## COSMIC compositor

### Protocol state

COSMIC also uses Smithay's `BackgroundEffectState`. Its handler in [`src/wayland/handlers/background_effect.rs`](https://github.com/pop-os/cosmic-comp/blob/5c93094574b31800caaa27255caac4a7253f24ad/src/wayland/handlers/background_effect.rs) converts the copied Smithay region into non-overlapping added rectangles minus subtracted rectangles, then stores the result in a separate double-buffered cached state.

It always advertises `Blur`; there is no dynamic renderer capability path. Protocol uniqueness, inert-object behavior, region copying, and destruction semantics come from Smithay.

### Rendering

COSMIC's implementation is in [`src/backend/render/wayland/blur_effect.rs`](https://github.com/pop-os/cosmic-comp/blob/5c93094574b31800caaa27255caac4a7253f24ad/src/backend/render/wayland/blur_effect.rs). It uses the same Smithay framebuffer-effect hook as niri, but its resource and damage choices differ.

`BlurElement` expands the capture geometry by a conservative radius chosen from a table of Dual-Kawase pass/offset combinations. The table uses support margins of 10, 20, 50, or 150 physical pixels. The original protocol region is translated into that expanded geometry and used only when drawing the result back.

At `capture_framebuffer()` it:

1. Allocates or reuses an `ABGR8888` capture texture for the expanded geometry.
2. Blits the active framebuffer into that texture.
3. Handles transformed outputs through an extra temporary render when a direct blit cannot express the transform.
4. Creates a second texture of the same full size.
5. Ping-pongs between the two textures while rendering successively smaller viewport regions for each downsample and upsample pass.
6. Stores the final full-size texture in Smithay's persistent effect cache.

The final draw clips to the exact requested rectangles, applies rounded-corner clipping, and adds fixed noise. Its shader normalizes by alpha when sampled input is partially transparent.

[`push_render_elements_from_surface_tree()`](https://github.com/pop-os/cosmic-comp/blob/5c93094574b31800caaa27255caac4a7253f24ad/src/backend/render/wayland/mod.rs) places the blur element immediately behind each corresponding Wayland surface, including subsurfaces. Smithay receives elements front-to-back and renders them in reverse, so the effect snapshots the scene before that surface is drawn.

### Damage and scanout

COSMIC relies on Smithay's generic framebuffer-effect dependency tracking. Because the element geometry includes the conservative support border, damage below anywhere in that expanded area causes recapture and repaint. A protocol-region or filter change increments the element commit counter and damages the visible effect area.

Smithay rejects direct scanout for framebuffer-effect elements. Image-copy capture uses COSMIC's normal scene construction and therefore includes blur.

### Lessons for Jay

COSMIC is the best source reference for explicitly separating the requested output region from a larger source-sampling region. Its conservative support table is simple and safe. Its temporary allocation on each recapture and full-texture clears are less attractive for Vulkan; Jay should cache output-sized ping-pong images.

## Hyprland

### Protocol state

Hyprland implements the protocol directly in [`src/protocols/BackgroundEffect.cpp`](https://github.com/hyprwm/Hyprland/blob/60695e18c8cd17399651a9af5db3f42d1867e1e6/src/protocols/BackgroundEffect.cpp). It keeps a copied pending region in the effect object and transfers it to `CWLSurface::m_blurRegion` from a surface commit listener. Effect destruction clears the pending region and waits for the next commit before removing committed state. A weak surface reference gives the required inert behavior, and both protocol errors are explicit.

It sends `Blur` unconditionally on bind. Region changes conservatively damage the entire global surface box on commit. This is correct but can repaint substantially more than necessary.

### Rendering

Hyprland has the most developed blur subsystem examined. It abstracts blur behind `IBlurProvider`, and render-pass elements report either `needsLiveBlur()` or `needsPrecomputeBlur()`.

The exact protocol region is applied in [`CHyprOpenGLImpl::renderTextureWithBlurInternal()`](https://github.com/hyprwm/Hyprland/blob/60695e18c8cd17399651a9af5db3f42d1867e1e6/src/render/OpenGL.cpp#L1910). It is clipped to surface dimensions, scaled to physical coordinates, transformed with the surface, and intersected with other compositor clip regions before the blurred background is drawn.

The default provider in [`src/render/gl/blur/Kawase.cpp`](https://github.com/hyprwm/Hyprland/blob/60695e18c8cd17399651a9af5db3f42d1867e1e6/src/render/gl/blur/Kawase.cpp) uses reusable output-sized work buffers rather than per-window textures. It:

1. Converts the compositor work-buffer color representation into an intermediate blur representation while applying brightness and contrast.
2. Expands source damage by the filter support radius.
3. Ping-pongs through configurable Dual-Kawase downsample and upsample passes.
4. Applies noise, brightness, vibrancy, and provider-specific material effects.
5. Converts the result back into the compositor work-buffer color representation.
6. Draws only requested output damage.

Live blur samples the current main framebuffer at the precise pass position. Precomputed or xray blur renders a full-monitor blur buffer once and reuses it for tiled windows or explicitly configured surfaces. Backdrop-scope markers capture a clean framebuffer before transformed window rendering so live blur does not accidentally include the current window.

### Damage

Hyprland calculates a Dual-Kawase support radius as:

```text
2 * blur_size * ((1 << passes) - 1) + material_sample_radius
```

Its render pass distinguishes final presentation damage from the larger region needed to execute blur. It intersects damaged pixels with live-blur bounds, expands that intersection for output damage, then expands again for source rendering. It also weakens opaque occlusion around live blur so source content remains available.

The code contains comments acknowledging that its expansion multipliers are still heuristic and that some moving-window edge cases produce artifacts. This confirms that partial blur damage is a real correctness problem even in a mature renderer.

### Lessons for Jay

Hyprland's shared output-sized work buffers and explicit color-space conversion are the closest architectural match for Jay's Vulkan blend-buffer plan. Its `needsLiveBlur` pass metadata also supports using a separate ordered path only when blur is present. Jay should use mathematically fixed support bounds rather than Hyprland's heuristic multipliers.

## Mutter

### Protocol state

Mutter's implementation is in [`src/wayland/meta-wayland-background-effect.c`](https://gitlab.gnome.org/GNOME/mutter/-/blob/e730d1e6fc006bf2e83371fd22c92160122c57ff/src/wayland/meta-wayland-background-effect.c). It uses normal `MetaWaylandSurfaceState` pending/current fields, copies `MtkRegion`, handles both protocol errors, clears pending state on effect destruction, and leaves an inert object after surface destruction.

It always advertises `Blur`. Before rendering, [`meta-wayland-actor-surface.c`](https://gitlab.gnome.org/GNOME/mutter/-/blob/e730d1e6fc006bf2e83371fd22c92160122c57ff/src/wayland/meta-wayland-actor-surface.c) clips the committed region to the surface size and passes it to the surface actor.

### Rendering

Mutter paints background effects immediately before surface actor content. The implementation in [`src/compositor/meta-background-effect.c`](https://gitlab.gnome.org/GNOME/mutter/-/blob/e730d1e6fc006bf2e83371fd22c92160122c57ff/src/compositor/meta-background-effect.c) transforms the effect region into stage coordinates, computes its bounding rectangle, expands that rectangle for filter support, and creates a `ClutterBlurNode` for the current stage framebuffer.

The blur node:

1. Blits the selected current framebuffer rectangle into an offscreen texture.
2. Applies `ClutterBlur`.
3. Draws the result only through rectangles from the requested effect region.
4. Applies saturation and deterministic screen-space noise in the final pipeline.
5. Modulates the effect by actor opacity.

[`ClutterBlur`](https://gitlab.gnome.org/GNOME/mutter/-/blob/e730d1e6fc006bf2e83371fd22c92160122c57ff/clutter/clutter/clutter-blur.c) implements a separable Gaussian. It dynamically downsamples by powers of two while scaled sigma is greater than six and both dimensions remain above 256 pixels. It then runs vertical and horizontal passes at the selected resolution. The shader combines adjacent Gaussian taps through linear interpolation and calculates kernel coefficients incrementally.

The compositor defaults are a radius of 24, saturation 1.25, and noise 0.015. Public compositor methods allow the shell to replace this policy. The source-sampling padding is conservatively `ceil(radius * 2)`.

### Damage

Each active blur installs a stage redraw-clip filter. If incoming redraw damage intersects the transformed source-sampling region, the filter unions the entire source-sampling region into the redraw clip. Changing a region damages both the previous and new source regions.

This is conservative but straightforward: Mutter does not attempt exact forward and reverse region propagation. It simply redraws the complete affected blur source whenever relevant lower content changes. This is a strong model for Jay's first correct implementation before optimizing partial blur passes.

Mutter skips the effect for non-2D actor transforms, clone painting, non-invertible transforms, or missing stage framebuffers. Normal output scale is incorporated into texture dimensions and blur radius.

### Lessons for Jay

Mutter provides the best reference for the separable-Gaussian option already proposed in Jay's plan. The dynamic downscale keeps a large radius affordable while retaining a finite, easy-to-model support region. Its conservative redraw filter is considerably easier to validate than exact dependency propagation.

## Smithay's shared framebuffer-effect design

Smithay is not itself a compositor, but its generic renderer changes are important because two production compositors use them.

`RenderElement` has `is_framebuffer_effect()` and `capture_framebuffer()` hooks. The damage tracker in [`src/backend/renderer/damage/mod.rs`](https://github.com/Smithay/smithay/blob/e3d461a057ba244d213a8498ec372b0799cca103/src/backend/renderer/damage/mod.rs) records element ordering. For each framebuffer effect it checks whether any damage introduced by lower elements overlaps the effect geometry. If so, it:

- Marks the effect cache for recapture.
- Damages the full effect geometry.
- Removes conflicting opaque occlusion so source pixels below are rendered.
- Calls `capture_framebuffer()` immediately before `draw()` during bottom-to-top execution.
- Persists an effect-specific `UserDataMap` across frames for cached textures.

The DRM compositor explicitly rejects scanout assignment for framebuffer-effect elements.

This abstraction is almost exactly the backend-neutral primitive Jay needs. Jay's `GfxApiOp::Blur` can be narrower because only blur is required, but its operation ordering and damage semantics should match Smithay's framebuffer-effect contract.

Smithay's protocol module also provides a useful baseline for object uniqueness, copied region state, inert objects, and commit synchronization. It only sends capabilities at bind time, however, so Jay still needs its planned manager-object tracking for dynamic Vulkan/OpenGL changes.

## Adjacent blur compositors without the protocol

### SwayFX and SceneFX

SwayFX revision [`87754778`](https://github.com/WillPower3309/swayfx/commit/877547783a5e8cb00d24704b6e56cc0974ae5568) creates SceneFX blur nodes below configured windows and layer surfaces. It has no `ext-background-effect-v1` server implementation; SceneFX issue [`#136`](https://github.com/wlrfx/scenefx/issues/136) tracks protocol support.

SceneFX revision [`ec7c1599`](https://github.com/wlrfx/scenefx/commit/ec7c1599d9b7b67d3772f8f3ec67ac0681d70853) uses Dual Kawase, reusable offscreen buffers, exact scene-node ordering, optional wallpaper-only optimized blur, and post-processing for brightness, contrast, saturation, and noise.

Its most interesting contribution is partial-damage artifact compensation. It expands original damage by the filter sample size, intersects that with each visible blur node, and expands again for the source padding. Before rerendering, it saves pixels in the padding outside real output damage. After rendering the enlarged region, it restores those pixels. This prevents unchanged higher-z content from being accidentally captured into a lower blur while avoiding full-output repaint.

That optimization is sophisticated and should be deferred in Jay. It is a useful future reference after a conservative implementation is correct.

### Wayfire

Wayfire revision [`1287ff2d`](https://github.com/WayfireWM/wayfire/commit/1287ff2d5ae48cea835793e2230e1526e8089980) has a compositor blur plugin but no `ext-background-effect-v1` implementation. The plugin offers Kawase, Gaussian, box, and bokeh algorithms through a common `wf_blur_base` interface.

Like SceneFX, Wayfire expands damage by the blur radius, snapshots the expanded framebuffer area, renders blur over that area, and saves/restores the padding outside original damage to avoid partial-update artifacts. Its default implementation is OpenGL-specific and does not operate on Wayfire's Vulkan backend.

Wayfire is useful evidence that a renderer-independent protocol layer does not make blur backend-independent automatically. Jay should advertise an empty capability set on OpenGL rather than expose a partially implemented effect.

## Testing observed in other projects

No dedicated `ext-background-effect-v1` protocol or screenshot tests were found in the test trees of the five exact implementations at the revisions examined. The source contains runtime warnings and defensive fallback paths, but protocol state, ordering, transformed regions, and lower-surface damage appear to depend primarily on manual testing.

Jay should not copy that gap. The planned software-renderer tests for double buffering, old/new regions, overlapping effects, transforms, and lower-background-only damage would provide stronger regression coverage than the implementations examined here.

## Recommendations for Jay

The research supports the overall plan, with the following refinements:

1. Keep blur as an ordered operation immediately before its surface. This is universal among implementations and is required for overlapping effects.
2. Keep production support Vulkan-only. There is no precedent requiring every renderer to expose the effect; dynamic capability reporting is the correct protocol mechanism.
3. Follow KWin's capability model: create the global unconditionally, advertise `BLUR` only after Vulkan shaders and required image formats are available, and broadcast changes when the render context changes.
4. Use a shared output-sized sampleable compositing target and cached ping-pong images, following Hyprland's resource model rather than KWin's per-window pyramids.
5. Represent the requested paint region separately from the expanded source-sampling region, as COSMIC and Mutter do.
6. Start with conservative damage: if lower damage intersects a blur's expanded sample region, recapture that source and repaint the complete requested effect region. This matches Smithay and Mutter and is easier to prove correct than minimal propagation.
7. Preserve separate rendering and presentation damage. The rendering region includes filter support; KMS and swapchain presentation damage only needs pixels whose final values changed.
8. Reject direct scanout whenever Jay emits a blur operation. This matches Smithay and avoids relying on opacity reasoning in the first implementation.
9. Run blur in Jay's floating-point Vulkan blend buffer and perform the existing output color conversion afterward. Hyprland's explicit color-space round trip and KWin's per-view render format show that blur must not bypass color management.
10. Keep the initial algorithm fixed. Dual Kawase is the established performance choice, but Mutter's separable Gaussian has simpler finite support and damage reasoning. Either is viable; for a correctness-first Vulkan implementation, the planned Gaussian remains defensible. A downscaled Gaussian like Mutter's can reduce cost without adopting a multi-level Kawase pyramid.
11. Cache all Vulkan images and descriptor state by render context and output size. None of the mature implementations allocates its complete blur resource set for every frame.
12. Include screenshot and capture paths through the same ordered operation stream. niri, COSMIC, and Mutter naturally gain correct capture behavior by integrating blur into their common scene renderer.

The closest conceptual references for Jay are Smithay for ordered framebuffer effects, Mutter for conservative Gaussian sampling and damage, KWin for capabilities and exact region masks, and Hyprland for output-sized work buffers and color-managed rendering. No one implementation solves Jay's Vulkan problem directly, but together they confirm that the proposed design is feasible.
