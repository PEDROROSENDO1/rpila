//! `[wallpaper]`: a picture behind everything, on every output, drawn by Pila
//! itself under the lower layer surfaces (a wallpaper program's Background
//! surface covers it).
//!
//! A new image (at startup, or a reload that changes `[wallpaper]`) is
//! decoded on a thread of its own (`decode`: never the event loop), shrunk to
//! what covers the largest output, and uploaded once to the renderer as a
//! texture when it's back, outside any frame; the decoded copy is dropped
//! then. Each frame only places that texture on each output (`placement`,
//! `Pila::scene_wallpaper`). A decode for an image no longer wanted (another
//! came since) is dropped. Not drawn while the session is locked.

use std::{cell::RefCell, collections::HashMap, path::PathBuf};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{element::Id, gles::GlesTexture, ContextId, ImportMem, Renderer},
    },
    reexports::calloop,
    utils::{Buffer, Logical, Point, Rectangle, Size},
};
use tracing::{info, warn};

use crate::{config::WallpaperFit, Pila};

/// The picture shown, as a texture of the renderer.
pub struct Shown {
    texture: GlesTexture,
    context_id: ContextId<GlesTexture>,
    size: Size<i32, Buffer>,
    fit: WallpaperFit,
    /// An element id per output (by name), new with each picture: each
    /// output's damage tracker sees an element of its own.
    ids: RefCell<HashMap<String, Id>>,
}

/// The wallpaper's state: what's shown, and which request a decode answers.
#[derive(Default)]
pub struct Wallpaper {
    shown: Option<Shown>,
    /// Bumped with each request: a decode for an older one is dropped.
    generation: u64,
}

/// What `Pila::scene` hands the renderer: the texture, and each output's
/// place, size and scale.
#[derive(Clone)]
pub struct SceneWallpaper {
    pub texture: GlesTexture,
    pub context_id: ContextId<GlesTexture>,
    pub size: Size<i32, Buffer>,
    pub fit: WallpaperFit,
    pub outputs: Vec<(Id, Rectangle<i32, Logical>, f64)>,
}

/// A decoded picture: RGBA rows, and its size.
struct Decoded {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

impl Pila {
    /// Show `[wallpaper]` as it is now: decode its image on a thread (none:
    /// nothing drawn, `[colors] background` shows).
    pub fn wallpaper_config_changed(&mut self) {
        self.wallpaper.generation += 1;
        let generation = self.wallpaper.generation;
        let Some(path) = self.config.wallpaper.image.clone() else {
            if self.wallpaper.shown.take().is_some() {
                self.queue_redraw();
            }
            return;
        };
        let fit = self.config.wallpaper.fit;
        let largest = self.largest_output();
        let (sender, channel) = calloop::channel::channel();
        let spawned = std::thread::Builder::new()
            .name("pila-wallpaper".into())
            .spawn(move || {
                let _ = sender.send(decode(&path, fit, largest));
            });
        if let Err(err) = spawned {
            warn!(?err, "can't load the wallpaper");
            return;
        }
        let inserted = self
            .loop_handle
            .insert_source(channel, move |event, _, state| {
                if let calloop::channel::Event::Msg(decoded) = event {
                    state.wallpaper_decoded(generation, fit, decoded);
                }
            });
        if let Err(err) = inserted {
            warn!(?err, "can't wait for the wallpaper");
        }
    }

    /// A decode is back: the texture, if it's still the one wanted.
    fn wallpaper_decoded(
        &mut self,
        generation: u64,
        fit: WallpaperFit,
        decoded: Result<Decoded, String>,
    ) {
        if generation != self.wallpaper.generation {
            return;
        }
        let decoded = match decoded {
            Ok(decoded) => decoded,
            Err(err) => {
                warn!(err, "no wallpaper");
                self.notify_error("The wallpaper can't be shown", &err);
                if self.wallpaper.shown.take().is_some() {
                    self.queue_redraw();
                }
                return;
            }
        };
        let Some(mut renderer) = self.backend.as_mut().and_then(|b| b.renderer()) else {
            return;
        };
        let size = Size::from((decoded.width as i32, decoded.height as i32));
        let texture = match renderer.import_memory(&decoded.rgba, Fourcc::Abgr8888, size, false) {
            Ok(texture) => texture,
            Err(err) => {
                warn!(?err, "can't upload the wallpaper");
                return;
            }
        };
        let context_id = renderer.context_id();
        drop(renderer);
        drop(decoded.rgba);
        // The decode's tens of megabytes are free now, but glibc keeps them
        // in its arena for the thread that made them: given back, so a new
        // picture doesn't leave the old one's behind.
        #[cfg(target_env = "gnu")]
        // SAFETY: malloc_trim only releases free memory.
        unsafe {
            libc::malloc_trim(0);
        }
        info!(
            width = decoded.width,
            height = decoded.height,
            "wallpaper loaded"
        );
        self.wallpaper.shown = Some(Shown {
            texture,
            context_id,
            size,
            fit,
            ids: RefCell::default(),
        });
        self.queue_redraw();
    }

    /// The largest output's size in physical pixels, each way; a 4K
    /// monitor's if there's none yet.
    fn largest_output(&self) -> (u32, u32) {
        let sizes = self.space.outputs().filter_map(|output| {
            let mode = output.current_mode()?;
            let size = output.current_transform().transform_size(mode.size);
            Some((size.w.max(1) as u32, size.h.max(1) as u32))
        });
        sizes
            .reduce(|a, b| (a.0.max(b.0), a.1.max(b.1)))
            .unwrap_or((3840, 2160))
    }

    /// The wallpaper for `Pila::scene`: none while there's no picture.
    pub(crate) fn scene_wallpaper(&self) -> Option<SceneWallpaper> {
        let shown = self.wallpaper.shown.as_ref()?;
        let mut ids = shown.ids.borrow_mut();
        let outputs = self
            .space
            .outputs()
            .filter_map(|output| {
                let rect = self.space.output_geometry(output)?;
                let id = ids.entry(output.name()).or_insert_with(Id::new).clone();
                Some((id, rect, output.current_scale().fractional_scale()))
            })
            .collect();
        Some(SceneWallpaper {
            texture: shown.texture.clone(),
            context_id: shown.context_id.clone(),
            size: shown.size,
            fit: shown.fit,
            outputs,
        })
    }
}

/// Read and decode the picture at `path`, as RGBA, shrunk (keeping its
/// shape, never grown) to what still covers `largest` one way or the other,
/// but at `Center`, shown at its size.
fn decode(path: &PathBuf, fit: WallpaperFit, largest: (u32, u32)) -> Result<Decoded, String> {
    let shown = path.display();
    let picture = image::ImageReader::open(path)
        .map_err(|err| format!("can't open {shown}: {err}"))?
        .with_guessed_format()
        .map_err(|err| format!("can't read {shown}: {err}"))?
        .decode()
        .map_err(|err| format!("can't decode {shown}: {err}"))?
        .into_rgba8();
    let size = picture.dimensions();
    let (width, height) = match fit {
        WallpaperFit::Center => size,
        _ => shrunk(size, largest),
    };
    let picture = if (width, height) == size {
        picture
    } else {
        image::imageops::resize(
            &picture,
            width,
            height,
            image::imageops::FilterType::Triangle,
        )
    };
    Ok(Decoded {
        rgba: picture.into_raw(),
        width,
        height,
    })
}

/// `size` shrunk to cover no more than `largest` needs, keeping its shape:
/// as small as it can be while still at least `largest` one way or the
/// other (so `Fill` crops from enough pixels); never grown.
pub fn shrunk(size: (u32, u32), largest: (u32, u32)) -> (u32, u32) {
    let (w, h) = (f64::from(size.0.max(1)), f64::from(size.1.max(1)));
    let scale = (f64::from(largest.0) / w)
        .max(f64::from(largest.1) / h)
        .min(1.0);
    (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    )
}

/// Where a picture of `image` pixels goes on an output of `output` (logical)
/// at `scale`: the part of the picture shown (its pixels) and where on the
/// output (logical, from its top-left).
pub fn placement(
    fit: WallpaperFit,
    image: Size<i32, Buffer>,
    output: Size<i32, Logical>,
    scale: f64,
) -> (Rectangle<f64, Logical>, Rectangle<i32, Logical>) {
    let (iw, ih) = (f64::from(image.w.max(1)), f64::from(image.h.max(1)));
    let (ow, oh) = (f64::from(output.w), f64::from(output.h));
    let whole_image = Rectangle::from_size((iw, ih).into());
    let whole_output = Rectangle::from_size(output);
    let centered = |w: f64, h: f64| -> Rectangle<i32, Logical> {
        let (w, h) = (w.round() as i32, h.round() as i32);
        Rectangle::new(
            ((output.w - w) / 2, (output.h - h) / 2).into(),
            (w, h).into(),
        )
    };
    match fit {
        WallpaperFit::Stretch => (whole_image, whole_output),
        WallpaperFit::Fill => {
            // Logical pixels a picture's pixel covers.
            let s = (ow / iw).max(oh / ih);
            let (w, h) = (ow / s, oh / s);
            let src = Rectangle::new(((iw - w) / 2.0, (ih - h) / 2.0).into(), (w, h).into());
            (src, whole_output)
        }
        WallpaperFit::Fit => {
            let s = (ow / iw).min(oh / ih);
            (whole_image, centered(iw * s, ih * s))
        }
        WallpaperFit::Center => {
            // A picture's pixel is a pixel of the output.
            let (w, h) = ((iw / scale).min(ow), (ih / scale).min(oh));
            let (sw, sh) = (w * scale, h * scale);
            let src = Rectangle::new(((iw - sw) / 2.0, (ih - sh) / 2.0).into(), (sw, sh).into());
            (src, centered(w, h))
        }
    }
}

impl SceneWallpaper {
    /// Each output's part of the wallpaper inside `visible` (global; every
    /// output's if `None`), for drawing at `origin` (global) and `scale`.
    pub fn elements(
        &self,
        origin: Point<i32, Logical>,
        scale: smithay::utils::Scale<f64>,
        visible: Option<Rectangle<i32, Logical>>,
    ) -> impl Iterator<
        Item = smithay::backend::renderer::element::texture::TextureRenderElement<GlesTexture>,
    > + '_ {
        self.outputs
            .iter()
            .filter(move |(_, rect, _)| visible.is_none_or(|area| area.overlaps(*rect)))
            .map(move |(id, rect, output_scale)| {
                let (src, dst) = placement(self.fit, self.size, rect.size, *output_scale);
                let at = (rect.loc + dst.loc - origin).to_f64().to_physical(scale);
                smithay::backend::renderer::element::texture::TextureRenderElement::from_static_texture(
                    id.clone(),
                    self.context_id.clone(),
                    at,
                    self.texture.clone(),
                    1,
                    smithay::utils::Transform::Normal,
                    None,
                    Some(src),
                    Some(dst.size),
                    None,
                    smithay::backend::renderer::element::Kind::Unspecified,
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rectangle as (x, y, width, height).
    type Rect<T> = (T, T, T, T);

    fn place(
        fit: WallpaperFit,
        image: (i32, i32),
        output: (i32, i32),
        scale: f64,
    ) -> (Rect<f64>, Rect<i32>) {
        let (src, dst) = placement(fit, image.into(), output.into(), scale);
        (
            (src.loc.x, src.loc.y, src.size.w, src.size.h),
            (dst.loc.x, dst.loc.y, dst.size.w, dst.size.h),
        )
    }

    #[test]
    fn fill_crops_the_picture_to_the_outputs_shape() {
        // A 4:3 picture on a 16:9 output: its top and bottom cut.
        let (src, dst) = place(WallpaperFit::Fill, (1600, 1200), (1920, 1080), 1.0);
        assert_eq!(dst, (0, 0, 1920, 1080));
        assert_eq!(src, (0.0, 150.0, 1600.0, 900.0));
    }

    #[test]
    fn fit_shows_the_whole_picture_centered() {
        let (src, dst) = place(WallpaperFit::Fit, (1600, 1200), (1920, 1080), 1.0);
        assert_eq!(src, (0.0, 0.0, 1600.0, 1200.0));
        assert_eq!(dst, (240, 0, 1440, 1080));
    }

    #[test]
    fn stretch_covers_the_output_out_of_shape() {
        let (src, dst) = place(WallpaperFit::Stretch, (100, 100), (1920, 1080), 1.0);
        assert_eq!(src, (0.0, 0.0, 100.0, 100.0));
        assert_eq!(dst, (0, 0, 1920, 1080));
    }

    #[test]
    fn center_shows_the_picture_at_its_size_cropped_where_larger() {
        // Smaller than the output, at scale 2: a picture's pixel is a pixel
        // of the output, half a logical one.
        let (src, dst) = place(WallpaperFit::Center, (400, 200), (1280, 720), 2.0);
        assert_eq!(src, (0.0, 0.0, 400.0, 200.0));
        assert_eq!(dst, (540, 310, 200, 100));
        // Larger: its middle.
        let (src, dst) = place(WallpaperFit::Center, (4000, 1000), (1920, 1080), 1.0);
        assert_eq!(dst, (0, 40, 1920, 1000));
        assert_eq!(src, (1040.0, 0.0, 1920.0, 1000.0));
    }

    #[test]
    fn a_picture_is_shrunk_to_cover_the_largest_output_never_grown() {
        assert_eq!(shrunk((7680, 4320), (3840, 2160)), (3840, 2160));
        assert_eq!(shrunk((6000, 4000), (2560, 1440)), (2560, 1707));
        assert_eq!(shrunk((1920, 1080), (2560, 1440)), (1920, 1080));
        assert_eq!(shrunk((0, 0), (1920, 1080)), (1, 1));
    }
}
