// SPDX-License-Identifier: GPL-3.0-or-later

//! Drawing frames into a caller-owned OpenGL framebuffer.
//!
//! mpv's render API does not create or own a GL context: the caller supplies
//! one, tells mpv how to resolve GL function pointers, and then asks for a
//! frame whenever it wants one. That fits a UI toolkit which already owns the
//! context and decides when to paint.
//!
//! This module deliberately stops at the framebuffer. It hands back the id it
//! was given and the size it rendered, and says nothing about textures,
//! images, or any toolkit type — wrapping the result is the caller's job, and
//! keeping that boundary here is what lets the crate stay UI-agnostic.
//!
//! # Threading
//!
//! [`RenderContext::render`] must run on the thread that owns the GL context.
//! [`Renderer::set_update_callback`] fires on an mpv thread instead, so the
//! callback may only wake the UI thread — never touch GL and never call back
//! into mpv.

use std::ffi::c_void;

use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};

use crate::{Player, Result};

/// Where a frame should be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// Framebuffer object id. `0` is the default framebuffer.
    pub fbo: i32,
    pub width: u32,
    pub height: u32,
    /// Whether to flip vertically.
    ///
    /// OpenGL's origin is bottom-left while most UI toolkits treat it as
    /// top-left, so a toolkit-owned framebuffer usually wants this on. Getting
    /// it wrong shows an upside-down picture rather than an error.
    pub flip_y: bool,
}

impl Target {
    /// A target with the flip most UI toolkits expect.
    #[must_use]
    pub fn new(fbo: i32, width: u32, height: u32) -> Self {
        Self {
            fbo,
            width,
            height,
            flip_y: true,
        }
    }

    #[must_use]
    pub fn flip_y(mut self, flip: bool) -> Self {
        self.flip_y = flip;
        self
    }

    /// Whether the target has any area to draw into.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// Renders frames from a [`Player`] into framebuffers the caller provides.
///
/// Borrows the player, so the player outlives every renderer made from it —
/// mpv's render context must not survive the mpv instance it belongs to.
pub struct Renderer<'player> {
    context: RenderContext<'player>,
}

impl<'player> Renderer<'player> {
    /// Creates a renderer against a live OpenGL context.
    ///
    /// `get_proc_address` resolves GL entry points by name; it is whatever the
    /// windowing layer provides (`eglGetProcAddress`, `glXGetProcAddress`, or
    /// the toolkit's own wrapper). `gl_context` is handed back to that
    /// function untouched.
    ///
    /// # Safety contract
    ///
    /// The GL context must be current on the calling thread, and must stay
    /// alive for as long as the renderer does. mpv will call
    /// `get_proc_address` during construction.
    pub fn new<C: 'static>(
        player: &'player Player,
        gl_context: C,
        get_proc_address: fn(&C, &str) -> *mut c_void,
    ) -> Result<Self> {
        let context = player.mpv().create_render_context(vec![
            RenderParam::ApiType(RenderParamApiType::OpenGl),
            RenderParam::InitParams(OpenGLInitParams {
                get_proc_address,
                ctx: gl_context,
            }),
            // Lets mpv time frames against the display instead of drawing
            // whenever asked, which is what makes interpolation meaningful.
            RenderParam::AdvancedControl(true),
        ])?;

        Ok(Self { context })
    }

    /// Draws the current frame into `target`.
    ///
    /// Must be called on the thread owning the GL context, with that context
    /// current. A target with no area is skipped rather than passed to mpv,
    /// which would reject it.
    pub fn render(&self, target: Target) -> Result<()> {
        if target.is_empty() {
            return Ok(());
        }

        // FlipY is a render parameter rather than an argument, so it is set
        // per frame alongside the framebuffer it applies to.
        self.context
            .set_parameter::<()>(RenderParam::FlipY(target.flip_y))?;
        self.context.render::<()>(
            target.fbo,
            i32::try_from(target.width).unwrap_or(i32::MAX),
            i32::try_from(target.height).unwrap_or(i32::MAX),
            target.flip_y,
        )?;
        Ok(())
    }

    /// Tells mpv the frame was presented.
    ///
    /// Call after the buffer swap. mpv uses the timing to keep audio and video
    /// in step; skipping it degrades interpolation and frame pacing.
    pub fn report_swap(&self) {
        self.context.report_swap();
    }

    /// Whether mpv has something new to draw.
    ///
    /// Lets a UI skip a repaint when nothing changed. Errors are reported as
    /// "redraw needed": a spurious repaint is cheaper than a frozen picture.
    pub fn needs_redraw(&self) -> bool {
        match self.context.update() {
            // MpvRenderUpdate is a plain bit field, not a bitflags type.
            Ok(update) => update & libmpv2::render::mpv_render_update::Frame != 0,
            Err(error) => {
                tracing::debug!(%error, "render update query failed; redrawing anyway");
                true
            }
        }
    }

    /// Registers a callback fired when a new frame is ready.
    ///
    /// Runs on an mpv thread. It must not call into mpv or touch GL — the only
    /// safe thing to do is wake the UI thread, which then calls
    /// [`Self::render`].
    pub fn set_update_callback<F: Fn() + Send + 'static>(&mut self, callback: F) {
        self.context.set_update_callback(callback);
    }
}

impl Player {
    /// Convenience wrapper over [`Renderer::new`].
    pub fn renderer<C: 'static>(
        &self,
        gl_context: C,
        get_proc_address: fn(&C, &str) -> *mut c_void,
    ) -> Result<Renderer<'_>> {
        Renderer::new(self, gl_context, get_proc_address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_default_to_the_flip_toolkits_expect() {
        let target = Target::new(3, 1920, 1080);
        assert!(target.flip_y);
        assert_eq!((target.fbo, target.width, target.height), (3, 1920, 1080));
    }

    #[test]
    fn flip_can_be_turned_off() {
        assert!(!Target::new(0, 640, 360).flip_y(false).flip_y);
    }

    #[test]
    fn a_zero_sized_target_is_empty() {
        assert!(Target::new(0, 0, 1080).is_empty());
        assert!(Target::new(0, 1920, 0).is_empty());
        assert!(!Target::new(0, 1920, 1080).is_empty());
    }
}
